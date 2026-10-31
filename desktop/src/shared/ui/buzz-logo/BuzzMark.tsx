/**
 * The static Cortex Max mark — a clock dial, matching the app icon — as a plain
 * SVG: no SMIL, no scripting, no animation machinery. It is rendered in
 * `currentColor` so it tints per theme and paints complete on the very first
 * frame. The 466x309 viewBox is kept from the original mark so every layout
 * slot that sizes the logo keeps its aspect ratio.
 */
export function BuzzMark({ className }: { className?: string }) {
  return (
    <svg
      aria-hidden="true"
      className={["buzz-mark", className].filter(Boolean).join(" ")}
      viewBox="0 0 466 309"
      fill="none"
    >
      <circle
        cx="233"
        cy="154.5"
        r="112"
        stroke="currentColor"
        strokeWidth="34"
      />
      <path
        d="M233 154.5 L233 74"
        stroke="currentColor"
        strokeWidth="28"
        strokeLinecap="round"
      />
      <path
        d="M233 154.5 L294 190"
        stroke="currentColor"
        strokeWidth="28"
        strokeLinecap="round"
      />
      <circle cx="233" cy="154.5" r="20" fill="currentColor" />
    </svg>
  );
}
