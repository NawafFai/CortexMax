import fs from "node:fs";
import path from "node:path";

/**
 * Catches upstream Buzz's visual identity surviving in a Cortex Max build.
 *
 * This guard exists because the identity leaked four separate times during one
 * rebrand, and each pass that swapped image files missed the next one. None of
 * the survivors were image files: the menu-bar bee was circles and rounded
 * rectangles evaluated per pixel in Rust, the old "C" ring was SVG written
 * inline in three React components, the theme fallbacks were hex literals in a
 * settings preview, and the browser sign-in page kept the bee as an SVG mask
 * inside a Rust string constant — under a heading that already read "Cortex
 * Max", which is how it survived a rename that read the text and not the
 * drawing. What all of them had in common is a greppable constant, so
 * constants are what this checks.
 *
 * Two deliberate limits:
 *
 * It does NOT check file or token names. `buzz.svg`, `buzz-wordmark.png`, the
 * `buzz-logo/` directory and the `--buzz-gradient-*` tokens keep their names
 * because renaming them churns imports across the app for no visual change —
 * what matters is what they draw, which the rules below cover. Nor does it
 * check identifiers that are contracts rather than branding: Builderlab's
 * `product=buzz` names this client's registration, and renaming it fails the
 * login.
 *
 * It scans only the desktop app, because that is what ships as Cortex Max.
 * `admin-web/` and `web/` are upstream's own deployed surfaces that this fork
 * does not build or publish; flagging them would leave the gate permanently
 * red with nothing to fix.
 */

/** Buzz's palette. Any of these in a shipped surface means the old look is back. */
const BUZZ_COLOURS = [
  // Chartreuse and its paired blue: the landing and sign-in surfaces.
  "#d7d72e",
  "#d7e7f6",
  "#8ee8f5",
  "#e9e94f",
  // The olive/navy wallpaper gradient.
  "#e6e6b6",
  "#c4d0da",
  "#4a4616",
  "#0a1423",
];

/**
 * Geometry of the retired marks, in every syntax the app writes SVG in: XML
 * attributes (Rust string constants, `.svg` files), JSX props, and packed
 * path data. The bee's wing circles and the "C" ring's open arc are the
 * distinctive coordinates — a mark that keeps them is the old mark.
 */
const RETIRED_GEOMETRY = [
  "bee-mask",
  'cx="91.7"',
  'cx="374.3"',
  "cx={91.7}",
  "cx={374.3}",
  "91.7, 154.5",
  "374.3, 154.5",
  "M323.4 78.7",
];

/** Identifiers that would reintroduce a retired mark by name. */
const RETIRED_SYMBOLS = ["tray_bee_icon", "bee_contains"];

/**
 * Where a shipped visual can hide. `tests` is included on purpose: the E2E
 * theme spec asserted the old gradient long after the theme had changed, so a
 * test is perfectly capable of holding the previous brand in place.
 */
const TARGETS = [
  { dir: "desktop/src", extensions: [".ts", ".tsx", ".css"] },
  { dir: "desktop/tests", extensions: [".ts"] },
  { dir: "desktop/src-tauri/src", extensions: [".rs"] },
  { dir: "desktop/public", extensions: [".svg", ".css", ".html"] },
  { dir: "desktop/src-tauri/icons", extensions: [".svg"] },
];

function walk(root, extensions, found = []) {
  let entries;
  try {
    entries = fs.readdirSync(root, { withFileTypes: true });
  } catch {
    // A target directory that does not exist is not a violation.
    return found;
  }
  for (const entry of entries) {
    if (entry.name === "node_modules" || entry.name === "target") continue;
    if (entry.name.startsWith(".")) continue;
    const full = path.join(root, entry.name);
    if (entry.isDirectory()) {
      walk(full, extensions, found);
    } else if (extensions.includes(path.extname(entry.name))) {
      found.push(full);
    }
  }
  return found;
}

/**
 * Scan `projectRoot` and return one finding per offence, each naming the file,
 * the line and the literal. "The brand check failed" without a location is the
 * report that made this take four passes in the first place.
 *
 * `overrides` holds `relative/path.ts:literal` keys, for the rare file that
 * needs to name an old value — a migration, or this checker's own fixtures.
 */
export function runBrandCheck(
  projectRoot,
  { overrides = new Set(), targets = TARGETS } = {},
) {
  const needles = [
    ...BUZZ_COLOURS.map((text) => ({ text, why: "a retired Buzz colour" })),
    ...RETIRED_GEOMETRY.map((text) => ({
      text,
      why: "geometry of a retired mark",
    })),
    ...RETIRED_SYMBOLS.map((text) => ({
      text,
      why: "a retired mark's identifier",
    })),
  ];

  const findings = [];
  for (const target of targets) {
    const root = path.join(projectRoot, target.dir);
    for (const file of walk(root, target.extensions)) {
      const relative = path.relative(projectRoot, file).split(path.sep).join("/");
      const lines = fs.readFileSync(file, "utf8").split("\n");
      lines.forEach((line, index) => {
        // Hex is written in both cases across the tree; the geometry needles
        // contain no letters, so folding case costs nothing there.
        const haystack = line.toLowerCase();
        for (const needle of needles) {
          if (!haystack.includes(needle.text.toLowerCase())) continue;
          if (overrides.has(`${relative}:${needle.text}`)) continue;
          findings.push({
            file: relative,
            line: index + 1,
            literal: needle.text,
            why: needle.why,
          });
        }
      });
    }
  }
  return findings;
}

/** Report findings the way the other repo gates do, and set the exit code. */
export function reportBrandCheck(findings, { scriptPath }) {
  if (findings.length === 0) return;
  console.error(
    `Desktop: found ${findings.length} piece(s) of the retired Buzz identity.\n` +
      "Cortex Max ships its own palette and mark; see desktop/src/shared/styles/globals/theme.css\n" +
      "and desktop/src/shared/ui/buzz-logo/BuzzMark.tsx for the current values.\n" +
      `A genuine need to name an old value can be allowlisted in ${scriptPath}.\n`,
  );
  for (const finding of findings) {
    console.error(
      `  ${finding.file}:${finding.line}: ${finding.literal} — ${finding.why}`,
    );
  }
  process.exit(1);
}
