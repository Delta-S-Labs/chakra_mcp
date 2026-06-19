import type { CSSProperties, ReactNode } from "react";
import styles from "./use-cases.module.css";

/**
 * Scroll-reveal wrapper. The reveal itself is pure CSS
 * (`animation-timeline: view()` in the stylesheet) — no JS, no
 * IntersectionObserver — so this stays a server component and, crucially,
 * the content is visible by default on engines that don't support
 * scroll-driven animations (it just skips the entrance). `style`/
 * `className` pass through so a Reveal can be the flex/grid child it
 * wraps. `delayMs` is accepted for call-site convenience but the timing
 * is CSS-driven, so it's intentionally not applied.
 */
export function Reveal({
  children,
  className = "",
  style,
}: {
  children: ReactNode;
  delayMs?: number;
  className?: string;
  style?: CSSProperties;
}) {
  return (
    <div className={`${styles.reveal} ${className}`.trim()} style={style}>
      {children}
    </div>
  );
}
