import styles from "./AnimatedMark.module.css";

/*
 * AnimatedMark
 *
 * Inline copy of public/brand/mark-composite.svg, with class hooks and
 * an SMIL <animateMotion> so the elements are reachable for CSS
 * animation. The static shape (every coord, every fill, every dash
 * pattern) matches the source SVG byte-for-byte — only className,
 * <style>/<filter>/<title>/<desc>/<animateMotion> were added. When
 * the user has prefers-reduced-motion: reduce, the CSS module strips
 * every animation and the rendered SVG is visually identical to the
 * source asset.
 *
 * Composition:
 *   - left  : 7-spoke hub with coral core + lime/butter satellites
 *   - center: brown arc from hub center to chakra core
 *   - right : layered halo + dashed orbit + coral chakra core
 *
 * Story: agents (satellites) feed the hub (dashes flow inward), the
 * hub relays a pulse along the arc (bright dot rides the path),
 * the chakra receives (halo + core flash on arrival).
 */

// Arc path is shared between the visible <path> and the
// <animateMotion mpath> ref so the dot rides the exact same curve as
// the rendered arc. mark-composite.svg uses: M 460 400 Q 650 300 760 400.
const ARC_PATH_D = "M 460 400 Q 650 300 760 400";

export default function AnimatedMark() {
  return (
    <div className={styles.wrap}>
      <svg
        xmlns="http://www.w3.org/2000/svg"
        viewBox="0 0 1200 800"
        role="img"
        aria-label="ChakraMCP composite mark — hub, relay arc, chakra. Animated to show inter-agent traffic."
        className={styles.svg}
      >
        <title>ChakraMCP composite mark — hub + chakra + relay arc</title>
        <desc>
          A hub on the left with seven dashed spokes radiating out to small
          satellite dots feeds a relay arc that connects to a chakra on the
          right. Animation: satellites pulse, dashes flow inward to the hub,
          a bright dot travels the arc, and the chakra halo and core flash
          on arrival.
        </desc>

        {/*
          Inline SVG filter used by the CSS stamp (filter: url(#stampGrunge)).
          Lives inside the same SVG document so the filter id resolves in
          all browsers without needing a separate fragment. Cheap to render
          — runs once on layout, not per frame.
        */}
        <defs>
          <filter id="stampGrunge" x="-10%" y="-10%" width="120%" height="120%">
            <feTurbulence type="fractalNoise" baseFrequency="0.9" numOctaves="2" seed="3" />
            <feDisplacementMap in="SourceGraphic" scale="2.4" />
          </filter>
        </defs>

        {/* Floating hub: 7-spoke node with a coral core and lime/butter satellites */}
        <g transform="translate(380 400)">
          <g
            className={styles.spokes}
            stroke="#5b3a2c"
            strokeWidth="6"
            strokeLinecap="round"
            strokeDasharray="10 14"
            opacity="0.55"
          >
            <line x1="0" y1="0" x2="0" y2="-180" />
            <line x1="0" y1="0" x2="155" y2="-90" />
            <line x1="0" y1="0" x2="155" y2="90" />
            <line x1="0" y1="0" x2="40" y2="190" />
            <line x1="0" y1="0" x2="-40" y2="190" />
            <line x1="0" y1="0" x2="-155" y2="90" />
            <line x1="0" y1="0" x2="-155" y2="-90" />
          </g>
          <g fill="#c5e060">
            <circle className={`${styles.satellite} ${styles["satellite-0"]}`} cx="0" cy="-180" r="22" />
            <circle className={`${styles.satellite} ${styles["satellite-1"]}`} cx="155" cy="90" r="22" />
            <circle className={`${styles.satellite} ${styles["satellite-2"]}`} cx="-40" cy="190" r="22" />
            <circle className={`${styles.satellite} ${styles["satellite-3"]}`} cx="-155" cy="-90" r="22" />
          </g>
          <g fill="#f5d34a">
            <circle
              className={`${styles.satellite} ${styles.satelliteButter} ${styles["satellite-4"]}`}
              cx="155"
              cy="-90"
              r="22"
            />
            <circle
              className={`${styles.satellite} ${styles.satelliteButter} ${styles["satellite-5"]}`}
              cx="40"
              cy="190"
              r="22"
            />
            <circle
              className={`${styles.satellite} ${styles.satelliteButter} ${styles["satellite-6"]}`}
              cx="-155"
              cy="90"
              r="22"
            />
          </g>
          <circle className={styles.hubCore} cx="0" cy="0" r="48" fill="#e35d4c" />
          <circle cx="0" cy="0" r="14" fill="#fbf3e3" />
        </g>

        {/* Chakra: layered halo, dashed orbit, coral core */}
        <g transform="translate(840 400)">
          <circle className={styles.chakraHalo} cx="0" cy="0" r="160" fill="#e35d4c" opacity="0.30" />
          <circle
            className={styles.chakraOrbit}
            cx="0"
            cy="0"
            r="110"
            fill="none"
            stroke="#e35d4c"
            strokeWidth="9"
            strokeDasharray="16 22"
          />
          <circle className={styles.chakraCore} cx="0" cy="0" r="50" fill="#e35d4c" />
        </g>

        {/* Relay arc connecting hub and chakra */}
        <path
          id="relayArcPath"
          d={ARC_PATH_D}
          stroke="#5b3a2c"
          strokeWidth="9"
          strokeLinecap="round"
          fill="none"
        />

        {/*
          Arc-traveling dot. <animateMotion mpath> rides the exact path
          rendered above. dur=3.4s; matches hubPulse / haloPulse /
          chakraCorePulse so the arrival flash lines up with the dot
          reaching the chakra. begin="0s" so all four animations share
          a phase. Hidden under prefers-reduced-motion via the CSS
          module (.arcDot display:none).
        */}
        <circle className={styles.arcDot} r="7" fill="#fbf3e3">
          <animateMotion dur="3.4s" repeatCount="indefinite" rotate="auto" begin="0s">
            <mpath href="#relayArcPath" />
          </animateMotion>
        </circle>
      </svg>
    </div>
  );
}

/*
 * Stamp — passport-style rubber stamp.
 *
 * Co-located with AnimatedMark because it shares the inline #stampGrunge
 * filter rendered by AnimatedMark's SVG. Place a <Stamp> in the same
 * subtree as <AnimatedMark /> (any descendant of <aside className=
 * "hero-board">) and the filter resolves.
 */
export function Stamp({ children }: { children: React.ReactNode }) {
  return <div className={styles.stamp}>{children}</div>;
}
