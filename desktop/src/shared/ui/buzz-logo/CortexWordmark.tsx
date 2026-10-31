/**
 * The Cortex Max wordmark, drawn rather than loaded.
 *
 * It replaces `landing/buzz-wordmark.png`, which had two faults a raster
 * wordmark cannot avoid. Its ink was baked in as black, so it could not follow
 * the theme the way the copy beneath it does — and it carried an opaque white
 * background, which on the amber landing rendered as a white block around the
 * name. Both are gone here: the text is `currentColor`, so it inherits
 * `text-foreground` in light and dark alike, and there is no background at all.
 *
 * `textLength` pins the mark to the full width of its box, so it occupies the
 * same space whether or not Inter has finished loading — no reflow on the very
 * screen that renders first. `lengthAdjust="spacing"` is deliberate over the
 * `spacingAndGlyphs` default: adjusting the letterforms themselves to hit a
 * target width distorts them, which in a wordmark is the one thing that reads
 * as wrong even to someone who could not say why.
 *
 * The viewBox is the cap height exactly. "Cortex Max" has no descenders, so
 * any more than that is padding baked into the asset, which is what made the
 * old PNG hard to place.
 */
export function CortexWordmark({ className }: { className?: string }) {
  return (
    <svg
      aria-label="Cortex Max"
      className={className}
      role="img"
      viewBox="0 0 600 80"
    >
      <text
        fill="currentColor"
        fontSize="108"
        fontWeight="800"
        lengthAdjust="spacing"
        letterSpacing="-0.05em"
        textLength="600"
        x="0"
        y="79"
      >
        Cortex Max
      </text>
    </svg>
  );
}
