# Pi adapter integration

Buzz uses the [buzz-pi-acp fork](https://github.com/salman1993/buzz-pi-acp).
Install Node.js 22 or newer, configure Pi, and install the latest development
adapter from the fork's `main` branch:

```sh
npm install -g @earendil-works/pi-coding-agent
pi
npm install -g --install-links=true 'git+https://github.com/salman1993/buzz-pi-acp.git#main'
```

This test setup intentionally tracks `main`; the Desktop runtime catalog pins a
reviewed adapter revision for users.

Make sure `pi` and `buzz-pi-acp` are on PATH, then restart Buzz.

## Tests

```sh
cargo test -p buzz-acp
```

## Codex project contract (macOS, opt-in)

`tests/codex_project_scope.rs` runs a real, explicitly selected Codex binary
against a localhost scripted model service. It creates a fresh synthetic HOME,
CODEX_HOME and project, forces file-only credential storage, clears inherited
environment values, and refuses known system config/requirements files. It does
not use an account, real provider, relay, or existing project. Plugin, app, hook,
browser, image and multi-agent features are disabled using the installed CLI's
feature names. External HTTP(S) uses a rejecting proxy; this is not an OS-level
egress guarantee for software that ignores proxy settings. Obtain authorization
before running; do not run under a second macOS Seatbelt sandbox because the
inner Codex sandbox then fails even the positive read case.

```sh
BUZZ_TEST_CODEX=/absolute/path/to/codex \
  cargo test --offline -p buzz-acp --test codex_project_scope -- --ignored --nocapture
```

The inventory case requires only the synthetic dynamic read tool and
`request_user_input`. The scope case deliberately enables the native shell in
the fixture only, then scripts a bounded actual model tool call. It requires an
inside read to succeed and sibling/symlink reads and writes to fail. On the
tested Codex 0.153.4, inventory passes but scope **fails**: the named read-only
profile is acknowledged yet the model tool reads a sibling and writes a file.
The regression is intentionally not converted into a passing known-bug test.
It blocks giving Codex builtin tools project access; it does not claim every Codex version
has this defect. Evidence remains in the printed synthetic temporary directory.

The explicit reasoning-only alternative is implemented in
`crates/buzz-agent/src/codex_project.rs`: actual Codex runs in a neutral temporary
directory with shell/patch/image/MCP/plugin/skill-loading surfaces disabled;
only Buzz's dynamic native project tools perform project operations. The exact
0.153.4 protocol/version acknowledgement is required. It never uses BuzzAgent's
API model backend, nor an ambient Codex profile. The desktop requires explicit
opt-in, an existing local managed identity, and separate scoped authentication.

Run the production ACP → Codex → native project broker regression with the
same synthetic-only authorization and proxy limitations described above:

```sh
BUZZ_TEST_CODEX=/absolute/path/to/codex \
  cargo test --offline -p buzz-agent --test project_workspace -- --include-ignored
```

This covers real binary tool inventory, scoped account absence, native read/create,
traversal/symlink rejection, forced shell/patch/image/MCP rejection, absent project
hints/path in model context, and stale generation fencing before provider dispatch.
It is not a live-provider, relay-backed, independent-review, or end-to-end task-board
acceptance test. The known-failing named-profile regression remains separate.

Run the ignored real-adapter test with a built fork checkout:

```sh
BUZZ_TEST_PI_ACP=/absolute/buzz-pi-acp/dist/index.js \
  cargo test -p buzz-acp real_pi_preserves -- --ignored
```
