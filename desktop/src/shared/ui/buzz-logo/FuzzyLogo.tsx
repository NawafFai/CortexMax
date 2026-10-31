import { cn } from "@/shared/lib/cn";
import BuzzLogoAnimation, {
  type BuzzLogoAnimationProps,
} from "./BuzzLogoAnimation";

export type FuzzyLogoProps = {
  /** When false, skips the looping feTurbulence texture filter and uses a CSS pulse instead. */
  fuzz?: boolean;
  className?: string;
  ariaLabel?: string;
  loop?: boolean;
  /** When looping, hide the mark for this many seconds between plays. */
  loopRestSeconds?: number;
  /** Set false when a parent drives its own opacity animation over the mark. */
  pulse?: boolean;
  reverse?: boolean;
  variant?: BuzzLogoAnimationProps["variant"];
};

/**
 * The Cortex Max mark with a lightweight CSS pulse. `fuzz` is accepted for
 * call-site compatibility; the mark is always the crisp geometry.
 */
export function FuzzyLogo({
  fuzz = true,
  className,
  ariaLabel = "Cortex Max logo",
  loop = false,
  loopRestSeconds = 0,
  pulse = true,
  reverse = false,
  variant = "v8",
}: FuzzyLogoProps) {
  // The mark no longer carries a texture animation, so the CSS pulse is the
  // only motion; a parent that drives its own opacity opts out via `pulse`.
  return (
    <BuzzLogoAnimation
      ariaLabel={ariaLabel}
      className={cn(pulse && "buzz-logo--pulse", className)}
      fullScreen={false}
      loop={loop}
      loopRestSeconds={loopRestSeconds}
      reverse={reverse}
      showBackground={false}
      textured={fuzz}
      variant={variant}
    />
  );
}
