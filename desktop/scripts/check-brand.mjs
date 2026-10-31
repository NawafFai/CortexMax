import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  reportBrandCheck,
  runBrandCheck,
} from "../../scripts/check-brand-core.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const projectRoot = path.resolve(__dirname, "../..");

// Cortex Max is a fork, so upstream's palette and marks keep arriving with
// every merge — and they arrive as constants in code, not as image files.
// This gate fails the build on the ones that have actually shipped by
// accident before.
const overrides = new Set([
  // `auth_complete_page_does_not_carry_the_retired_mark` has to name the mark
  // it is asserting the absence of. Exempting the file here loses nothing:
  // that test reads the page constant itself, so it is a stricter guard on
  // this one file than a text scan could be.
  "desktop/src-tauri/src/builderlab.rs:bee-mask",
]);

reportBrandCheck(runBrandCheck(projectRoot, { overrides }), {
  scriptPath: "desktop/scripts/check-brand.mjs",
});
