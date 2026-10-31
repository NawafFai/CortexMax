/**
 * The animated Cortex Max mark used on the loading gates: a clock whose hands
 * sweep around a still dial. Geometry is identical to the static
 * {@link BuzzMark}, rendered in `currentColor` so it tints per theme.
 *
 * The hands revolve, not the dial. That is not a style choice: a complete
 * circle rotating about its own centre is indistinguishable from a stationary
 * one, so animating the dial would run an animation nobody can see. The
 * earlier mark was an open arc, where the same rotation read clearly — this is
 * what changed when the arc became a ring.
 *
 * Each moving part is its own HTML-level `<svg>` layer and the revolution
 * animates that element's CSS transform. This is deliberate: WebKit paints SVG
 * *children* on the main thread, so a transform animation on a `<path>` freezes
 * for as long as boot work (bundle eval, first React render of the app tree)
 * hogs the thread — exactly the window in which the loading gate is on screen.
 * Transforms on HTML-level elements run on the compositor (Core Animation in
 * WKWebView) and keep moving regardless (see animations.css).
 *
 * Everything is plain SVG + CSS (no JS/SMIL), so it paints on the very first
 * frame and the motion starts as soon as styles load. Reduced motion falls back
 * to the static silhouette via the CSS media query.
 */
export function FlappingBee({ className }: { className?: string }) {
  return (
    <div
      aria-hidden="true"
      className={[
        "buzz-mark",
        "bee-sprite",
        "relative",
        "aspect-[466/309]",
        className,
      ]
        .filter(Boolean)
        .join(" ")}
    >
      {/* The dial and hub stay put. */}
      <svg
        aria-hidden="true"
        className="absolute inset-0 block h-full w-full"
        fill="none"
        viewBox="0 0 466 309"
      >
        <circle
          cx="233"
          cy="154.5"
          r="112"
          stroke="currentColor"
          strokeWidth="34"
        />
        <circle cx="233" cy="154.5" r="20" fill="currentColor" />
      </svg>

      {/* Minute hand: the fast sweep that carries the motion. */}
      <svg
        aria-hidden="true"
        className="cm-ring-layer absolute inset-0 block h-full w-full"
        fill="none"
        viewBox="0 0 466 309"
      >
        <path
          d="M233 154.5 L233 74"
          stroke="currentColor"
          strokeLinecap="round"
          strokeWidth="28"
        />
      </svg>

      {/* Hour hand, slower, so the two are rarely aligned and the mark keeps
          changing shape as it turns. */}
      <svg
        aria-hidden="true"
        className="cm-hand-hour relative block h-full w-full"
        fill="none"
        viewBox="0 0 466 309"
      >
        <path
          d="M233 154.5 L294 190"
          stroke="currentColor"
          strokeLinecap="round"
          strokeWidth="28"
        />
      </svg>
    </div>
  );
}
