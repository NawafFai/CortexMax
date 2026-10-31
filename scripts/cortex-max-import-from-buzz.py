#!/usr/bin/env python3
"""Bring the owner's Buzz team into Cortex Max (one-time, owner-initiated).

Cortex Max is an independent build of Buzz with its own data directories.
This script copies the NON-SECRET state an installed Buzz.app holds about the
owner's managed agents, so the same agents boot inside Cortex Max:

  Buzz (read-only source)                         -> Cortex Max
  ~/Library/Application Support/xyz.block.buzz.app   ~/Library/Application Support/com.cortexmax.app
      agents/managed-agents.json (records)             agents/managed-agents.json
      agents/teams.json, personas.json, global-agent-config.json, teams/, retention/
      tts-settings.json
  ~/.buzz (agent nest: AGENTS.md, knowledge, skills)  ~/.cortex-max
  ~/Library/Application Support/Buzz (node tools)     ~/Library/Application Support/Cortex Max

Private keys are NOT handled here. The records are written without keys and a
trigger file is left beside them; on its next boot Cortex Max itself asks
macOS for permission to read the installed Buzz.app's keychain item and copies
each agent's key into its own keychain item (see
`import_agent_keys_from_buzz` in desktop/src-tauri/src/managed_agents/storage.rs).
Credential files inside the nest (tool logins) are skipped; sign in to those
tools again inside Cortex Max if an agent needs them.

Nothing under Buzz is modified. Run with Cortex Max closed. After the import,
quit Buzz before launching Cortex Max: both apps would otherwise run the same
agent identities at the same time and duplicate every agent on the relay.

Usage: python3 scripts/cortex-max-import-from-buzz.py [--dry-run]
"""
from __future__ import annotations

import json
import os
import subprocess
import sys
import tempfile
import time

HOME = os.path.expanduser("~")
SUPPORT = os.path.join(HOME, "Library", "Application Support")
SRC_APP = os.path.join(SUPPORT, "xyz.block.buzz.app")
DST_APP = os.path.join(SUPPORT, "com.cortexmax.app")
SRC_NEST = os.path.join(HOME, ".buzz")
DST_NEST = os.path.join(HOME, ".cortex-max")
SRC_TOOLS = os.path.join(SUPPORT, "Buzz")
DST_TOOLS = os.path.join(SUPPORT, "Cortex Max")
IMPORT_TRIGGER = ".import-keys-from-buzz"
RECORD_FILES = ("teams.json", "personas.json", "global-agent-config.json")
NEST_EXCLUDES = [
    ".nest-agents-version",  # let Cortex Max refresh its own AGENTS.md template
    "auth.json",
    "*.token",
    "credentials*",
    ".env",
    "*.pem",
]
PATH_REWRITES = [
    (SRC_TOOLS + "/", DST_TOOLS + "/"),
    (SRC_NEST + "/", DST_NEST + "/"),
    ("/Applications/Buzz.app/Contents/MacOS/", "/Applications/Cortex Max.app/Contents/MacOS/"),
]

DRY_RUN = "--dry-run" in sys.argv[1:]


def say(msg: str) -> None:
    print(msg, flush=True)


def die(msg: str) -> None:
    print(f"ERROR: {msg}", file=sys.stderr, flush=True)
    sys.exit(1)


def running(pattern: str) -> bool:
    result = subprocess.run(["pgrep", "-f", pattern], capture_output=True, text=True)
    return bool(result.stdout.strip())


def write_private(path: str, data: str) -> None:
    """Write `data` to `path` atomically with mode 0600 (never through a symlink)."""
    if DRY_RUN:
        say(f"  [dry-run] write {path}")
        return
    directory = os.path.dirname(path)
    os.makedirs(directory, exist_ok=True)
    if os.path.islink(path):
        die(f"refusing to write through a symlink: {path}")
    fd, tmp = tempfile.mkstemp(prefix=".import-", dir=directory)
    try:
        os.fchmod(fd, 0o600)
        with os.fdopen(fd, "w") as handle:
            handle.write(data)
            handle.flush()
            os.fsync(handle.fileno())
        os.replace(tmp, path)
    except Exception:
        try:
            os.unlink(tmp)
        except OSError:
            pass
        raise


def rewrite_paths(value):
    if isinstance(value, str):
        for old, new in PATH_REWRITES:
            if old in value:
                value = value.replace(old, new)
        return value
    if isinstance(value, list):
        return [rewrite_paths(item) for item in value]
    if isinstance(value, dict):
        return {key: rewrite_paths(item) for key, item in value.items()}
    return value


def rsync(src: str, dst: str, excludes: list[str] | None = None) -> None:
    if not os.path.isdir(src):
        say(f"  skip (absent): {src}")
        return
    args = ["rsync", "-a", "--links"]
    for pattern in excludes or []:
        args += ["--exclude", pattern]
    args += [src.rstrip("/") + "/", dst.rstrip("/") + "/"]
    if DRY_RUN:
        say(f"  [dry-run] {' '.join(args)}")
        return
    os.makedirs(dst, exist_ok=True)
    subprocess.run(args, check=True)
    say(f"  copied {src} -> {dst}")


def main() -> None:
    say("[1/5] Preflight")
    if running("Cortex Max.app/Contents/MacOS/buzz-desktop"):
        die("Cortex Max is running. Quit it first; the import rewrites its agents directory.")
    src_records = os.path.join(SRC_APP, "agents", "managed-agents.json")
    if not os.path.isfile(src_records):
        die(f"Buzz agent records not found: {src_records}")
    if os.path.islink(SRC_APP) or os.path.islink(DST_APP):
        die("refusing: an app data directory is a symlink")
    if running("Buzz.app/Contents/MacOS/buzz-desktop"):
        say("  Buzz is running: allowed (read-only source). Quit Buzz BEFORE launching Cortex Max.")
    say(f"  source {SRC_APP}")
    say(f"  target {DST_APP}")

    say("[2/5] Agent records (without keys)")
    with open(src_records) as handle:
        records = json.load(handle)
    if not isinstance(records, list):
        die("managed-agents.json is not a list of records")
    imported = []
    for record in records:
        record = dict(record)
        record.pop("runtime_pid", None)
        record.pop("private_key_nsec", None)
        imported.append(rewrite_paths(record))
    say(f"  records: {len(imported)}")

    dst_agents = os.path.join(DST_APP, "agents")
    stamp = time.strftime("%Y%m%d-%H%M%S")
    if os.path.isdir(dst_agents):
        backup = f"{dst_agents}.before-import-{stamp}"
        if DRY_RUN:
            say(f"  [dry-run] move existing {dst_agents} -> {backup}")
        else:
            os.rename(dst_agents, backup)
            say(f"  existing agents dir kept at {backup}")
    if not DRY_RUN:
        os.makedirs(dst_agents, mode=0o700, exist_ok=True)
    write_private(os.path.join(dst_agents, "managed-agents.json"), json.dumps(imported, indent=2) + "\n")
    for name in RECORD_FILES:
        src = os.path.join(SRC_APP, "agents", name)
        if os.path.isfile(src):
            with open(src) as handle:
                write_private(os.path.join(dst_agents, name), json.dumps(rewrite_paths(json.load(handle)), indent=2) + "\n")
    for name in ("teams", "retention"):
        rsync(os.path.join(SRC_APP, "agents", name), os.path.join(dst_agents, name))
    tts = os.path.join(SRC_APP, "tts-settings.json")
    if os.path.isfile(tts):
        with open(tts) as handle:
            write_private(os.path.join(DST_APP, "tts-settings.json"), handle.read())
    write_private(os.path.join(dst_agents, IMPORT_TRIGGER), f"requested {stamp}\n")
    say("  trigger written: Cortex Max copies the agent keys from the Buzz keychain item on its next boot")

    say("[3/5] Agent nest")
    rsync(SRC_NEST, DST_NEST, excludes=NEST_EXCLUDES)

    say("[4/5] Managed node tools (ACP adapters)")
    if os.path.isdir(DST_TOOLS):
        say(f"  already present: {DST_TOOLS} (left as is)")
    elif os.path.isdir(SRC_TOOLS):
        if DRY_RUN:
            say(f"  [dry-run] cp -Rc {SRC_TOOLS} {DST_TOOLS}")
        else:
            subprocess.run(["cp", "-Rc", SRC_TOOLS, DST_TOOLS], check=True)
            say(f"  cloned {SRC_TOOLS} -> {DST_TOOLS}")
    else:
        say("  no managed node tools to copy")

    say("[5/5] Done")
    say("  Quit Buzz, then launch Cortex Max. Approve the one macOS dialog that asks to read the")
    say("  Buzz keychain item; after that boot the agents run with their own keys inside Cortex Max.")
    say("  Buzz's own data was not modified.")


if __name__ == "__main__":
    main()
