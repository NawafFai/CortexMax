import assert from "node:assert/strict";
import { mkdirSync, mkdtempSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import test from "node:test";
import { runBrandCheck } from "./check-brand-core.mjs";

/** Build a throwaway tree with `files` written under it. */
function fixture(files) {
  const root = mkdtempSync(path.join(tmpdir(), "brand-check-"));
  for (const [relative, content] of Object.entries(files)) {
    const full = path.join(root, relative);
    mkdirSync(path.dirname(full), { recursive: true });
    writeFileSync(full, content);
  }
  return root;
}

test("a clean tree reports nothing", () => {
  const root = fixture({
    "desktop/src/theme.css": ":root { --primary: 28 91% 54%; }",
    "desktop/src-tauri/src/tray.rs": "fn tray_clock_icon() {}",
  });
  assert.deepEqual(runBrandCheck(root), []);
});

test("catches a retired colour and names where it is", () => {
  const root = fixture({
    "desktop/src/theme.css": "body { background: #d7d72e; }",
  });
  const findings = runBrandCheck(root);
  assert.equal(findings.length, 1);
  assert.equal(findings[0].file, "desktop/src/theme.css");
  assert.equal(findings[0].line, 1);
  assert.equal(findings[0].literal, "#d7d72e");
});

// The reason this checker exists: the sign-in page said "Cortex Max"
// everywhere in text and still drew upstream's bee, so a scan that trusts
// renamed strings would have passed it.
test("catches the retired mark even when every string is rebranded", () => {
  const root = fixture({
    "desktop/src-tauri/src/builderlab.rs": [
      'const PAGE: &str = r#"<svg aria-label="Cortex Max">',
      '  <mask id="bee-mask"><circle cx="91.7" cy="154.5" r="91.7"/></mask>',
      '</svg>"#;',
    ].join("\n"),
  });
  const literals = runBrandCheck(root).map((f) => f.literal);
  assert.ok(literals.includes("bee-mask"));
  assert.ok(literals.includes('cx="91.7"'));
});

// The same mark is written three ways across the app; missing any one syntax
// is how a leak survives a pass.
test("catches the mark in JSX and path-data syntax, not just XML", () => {
  const root = fixture({
    "desktop/src/Mark.tsx": "<circle cx={91.7} cy={154.5} r={91.7} />",
    "desktop/src/logo.css": '.a { d: path("M323.4 78.7 A118 118 0 1 0 1 1"); }',
    "desktop/public/mark.svg": '<circle cx="374.3" cy="154.5" r="91.7"/>',
  });
  const files = new Set(runBrandCheck(root).map((f) => f.file));
  assert.deepEqual(
    [...files].sort(),
    ["desktop/public/mark.svg", "desktop/src/Mark.tsx", "desktop/src/logo.css"],
  );
});

// A test that asserts the old gradient holds the old brand in place just as
// firmly as a stylesheet does.
test("scans tests, because a test can pin the previous brand", () => {
  const root = fixture({
    "desktop/tests/e2e/theme.spec.ts": 'expect(top).toBe("#e6e6b6");',
  });
  assert.equal(runBrandCheck(root).length, 1);
});

test("hex matching ignores case", () => {
  const root = fixture({ "desktop/src/a.tsx": 'const c = "#D7D72E";' });
  assert.equal(runBrandCheck(root).length, 1);
});

test("an override exempts one literal in one file, and only there", () => {
  const root = fixture({
    "desktop/src/a.tsx": '"#d7d72e"',
    "desktop/src/b.tsx": '"#d7d72e"',
  });
  const findings = runBrandCheck(root, {
    overrides: new Set(["desktop/src/a.tsx:#d7d72e"]),
  });
  assert.equal(findings.length, 1);
  assert.equal(findings[0].file, "desktop/src/b.tsx");
});

// Upstream's own web surfaces are not built or published by this fork, so
// scanning them would leave the gate permanently red over nothing.
test("leaves surfaces the fork does not ship alone", () => {
  const root = fixture({
    "admin-web/src/styles.css": "background: #d7d72e;",
    "web/src/Invite.tsx": '"#D7E7F6"',
  });
  assert.deepEqual(runBrandCheck(root), []);
});

test("a missing target directory is not a violation", () => {
  assert.deepEqual(runBrandCheck(fixture({})), []);
});
