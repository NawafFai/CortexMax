import type { CSSProperties } from "react";
import "./buzz-logo-animation.css";

type VariantKey = "v1" | "v2" | "v3" | "v4" | "v5" | "v6" | "v7" | "v8";

export type BuzzLogoAnimationProps = {
  ariaLabel?: string;
  className?: string;
  fullScreen?: boolean;
  /** A looping mark breathes with the CSS pulse; the Cortex Max mark does not morph. */
  loop?: boolean;
  /** Accepted for call-site compatibility; the Cortex Max mark does not morph. */
  loopRestSeconds?: number;
  /** Accepted for call-site compatibility; the Cortex Max mark does not morph. */
  reverse?: boolean;
  showBackground?: boolean;
  style?: CSSProperties;
  /** Accepted for call-site compatibility; there is no texture filter to skip. */
  textured?: boolean;
  /** Accepted for call-site compatibility; every variant renders the same mark. */
  variant?: VariantKey;
};

/**
 * The Cortex Max mark in every slot the animated Buzz morph used to fill.
 * The props are kept so call sites compile unchanged. There is no SMIL and no
 * texture filter: the mark is plain SVG in `currentColor`, and a looping mark
 * breathes with the lightweight `buzz-logo--pulse` CSS animation.
 */
export default function BuzzLogoAnimation(props: BuzzLogoAnimationProps) {
  const {
    ariaLabel = "Cortex Max logo",
    className = "",
    fullScreen = true,
    loop = false,
    showBackground = true,
    style,
  } = props;
  const classes = [
    "buzz-logo",
    fullScreen && "buzz-logo--screen",
    !fullScreen && "buzz-logo--compact",
    showBackground && "buzz-logo--background",
    loop && "buzz-logo--pulse",
    className,
  ]
    .filter(Boolean)
    .join(" ");

  return (
    <div className={classes} style={style} role="img" aria-label={ariaLabel}>
      <svg
        className="buzz-logo__mark"
        viewBox="0 0 466 309"
        width="466"
        height="309"
        aria-hidden="true"
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
    </div>
  );
}
