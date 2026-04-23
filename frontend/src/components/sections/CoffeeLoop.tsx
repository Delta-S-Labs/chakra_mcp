import styles from "./CoffeeLoop.module.css";

/**
 * (C) Video loop — coffee-shop supply chain.
 *
 * Six-second seamless loop:
 *   0.0–1.0s  Night. Coffee shop flags low inventory (red pulse).
 *   1.0–2.0s  Three signal arcs shoot out in parallel to bakery, produce, mill.
 *   2.0–3.0s  Three green checks return. Coffee shop pulse turns green.
 *   3.0–4.0s  Sky transitions night → dawn → morning. Clock advances.
 *   4.0–5.0s  Morning. OPEN sign lights up, steam curls from shop.
 *   5.0–6.0s  Sky dims back to night. Loop.
 *
 * Rendered in pure SVG + CSS keyframes — no JS animation loop.
 */
export default function CoffeeLoop() {
  return (
    <section className={styles.section} aria-label="Coffee shop supply chain at 3am">
      <div className={styles.header}>
        <div className={styles.eyebrow}>Example C · Video loop</div>
        <h2 className={styles.headline}>Agents don&apos;t keep office hours.</h2>
        <p className={styles.body}>
          At 3am, a corner coffee shop&apos;s ordering agent notices inventory running low. It
          pings the bakery, the produce supplier, and the coffee mill in parallel. By 6am all the
          paperwork is done. The owner opens at 7am to a stocked café.
        </p>
      </div>

      <div className={styles.stage}>
        <svg
          className={styles.scene}
          viewBox="0 0 1200 560"
          preserveAspectRatio="xMidYMid slice"
          role="img"
          aria-label="Animated scene of four shops at night, signals flowing between them, then morning arriving"
        >
          {/* Sky — animates night → dawn → morning → night */}
          <rect className={styles.sky} x="0" y="0" width="1200" height="360" />

          {/* Grid texture */}
          <g className={styles.grid} opacity="0.22">
            {Array.from({ length: 15 }).map((_, i) => (
              <line key={`v${i}`} x1={i * 88} y1="0" x2={i * 88} y2="560" stroke="currentColor" strokeWidth="1" />
            ))}
            {Array.from({ length: 7 }).map((_, i) => (
              <line key={`h${i}`} x1="0" y1={i * 88} x2="1200" y2={i * 88} stroke="currentColor" strokeWidth="1" />
            ))}
          </g>

          {/* Moon */}
          <g className={styles.moon}>
            <circle cx="120" cy="90" r="38" fill="#fef0c8" />
            <circle cx="135" cy="82" r="34" fill="url(#skyDark)" />
          </g>

          {/* Sun */}
          <g className={styles.sun}>
            <circle cx="1080" cy="110" r="42" fill="#f7b85c" />
            <g className={styles.sunRays} stroke="#f7b85c" strokeWidth="4" strokeLinecap="round">
              <line x1="1080" y1="30" x2="1080" y2="56" />
              <line x1="1080" y1="164" x2="1080" y2="190" />
              <line x1="1000" y1="110" x2="1026" y2="110" />
              <line x1="1134" y1="110" x2="1160" y2="110" />
              <line x1="1026" y1="56" x2="1044" y2="74" />
              <line x1="1116" y1="146" x2="1134" y2="164" />
              <line x1="1134" y1="56" x2="1116" y2="74" />
              <line x1="1044" y1="146" x2="1026" y2="164" />
            </g>
          </g>

          {/* Ground strip */}
          <rect x="0" y="360" width="1200" height="200" fill="url(#groundGrad)" />
          <line x1="0" y1="360" x2="1200" y2="360" stroke="rgba(26,20,16,0.25)" strokeWidth="1.5" />

          {/* Gradient defs */}
          <defs>
            <linearGradient id="skyDark" x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stopColor="#0b1128" />
              <stop offset="100%" stopColor="#2a2f5a" />
            </linearGradient>
            <linearGradient id="groundGrad" x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stopColor="#c9bfa6" />
              <stop offset="100%" stopColor="#dfd5b8" />
            </linearGradient>
            <linearGradient id="coralGlow" x1="0" y1="0" x2="0" y2="1">
              <stop offset="0%" stopColor="#ec6a52" stopOpacity="0.95" />
              <stop offset="100%" stopColor="#ec6a52" stopOpacity="0" />
            </linearGradient>
          </defs>

          {/* Signal arcs — coffee shop (300, 420) to each target */}
          {/* Arc to bakery (600, 420) */}
          <path
            className={`${styles.arc} ${styles.arcOut1}`}
            d="M 300 400 Q 450 260 600 400"
            stroke="var(--accent-coral, #ec6a52)"
            strokeWidth="3"
            fill="none"
            strokeLinecap="round"
          />
          <path
            className={`${styles.arc} ${styles.arcBack1}`}
            d="M 600 400 Q 450 260 300 400"
            stroke="oklch(78% 0.16 140)"
            strokeWidth="3"
            fill="none"
            strokeLinecap="round"
          />

          {/* Arc to produce (830, 420) */}
          <path
            className={`${styles.arc} ${styles.arcOut2}`}
            d="M 300 400 Q 565 200 830 400"
            stroke="var(--accent-coral, #ec6a52)"
            strokeWidth="3"
            fill="none"
            strokeLinecap="round"
          />
          <path
            className={`${styles.arc} ${styles.arcBack2}`}
            d="M 830 400 Q 565 200 300 400"
            stroke="oklch(78% 0.16 140)"
            strokeWidth="3"
            fill="none"
            strokeLinecap="round"
          />

          {/* Arc to mill (1040, 420) */}
          <path
            className={`${styles.arc} ${styles.arcOut3}`}
            d="M 300 400 Q 670 160 1040 400"
            stroke="var(--accent-coral, #ec6a52)"
            strokeWidth="3"
            fill="none"
            strokeLinecap="round"
          />
          <path
            className={`${styles.arc} ${styles.arcBack3}`}
            d="M 1040 400 Q 670 160 300 400"
            stroke="oklch(78% 0.16 140)"
            strokeWidth="3"
            fill="none"
            strokeLinecap="round"
          />

          {/* Shops */}
          {/* Coffee shop — center hero */}
          <g className={`${styles.shop} ${styles.coffeeShop}`} transform="translate(220, 340)">
            {/* roof */}
            <path d="M 0 40 L 80 0 L 160 40 Z" fill="#3e2a20" />
            {/* body */}
            <rect x="8" y="40" width="144" height="80" fill="#5a3e2e" stroke="#3e2a20" strokeWidth="2" />
            {/* door */}
            <rect x="64" y="68" width="32" height="52" fill="#2a1b13" rx="2" />
            {/* window */}
            <rect x="20" y="56" width="30" height="30" fill="#fce49a" opacity="0.85" />
            <rect x="110" y="56" width="30" height="30" fill="#fce49a" opacity="0.85" />
            {/* sign */}
            <rect className={styles.openSign} x="30" y="22" width="100" height="18" fill="#ec6a52" rx="3" />
            <text className={styles.openSignText} x="80" y="35" textAnchor="middle" fontSize="11" fontWeight="700" fill="#fef6e0" fontFamily="system-ui, sans-serif">
              COFFEE
            </text>
            {/* status dot */}
            <circle className={styles.statusRed} cx="80" cy="-8" r="7" fill="#ec6a52" />
            <circle className={styles.statusGreen} cx="80" cy="-8" r="7" fill="oklch(72% 0.18 140)" />
            {/* steam */}
            <g className={styles.steam}>
              <path d="M 96 8 q 4 -10 0 -20 q -4 -10 0 -20" stroke="#d8c9a8" strokeWidth="3" fill="none" strokeLinecap="round" />
              <path d="M 80 4 q 4 -10 0 -20 q -4 -10 0 -20" stroke="#d8c9a8" strokeWidth="3" fill="none" strokeLinecap="round" />
              <path d="M 64 8 q 4 -10 0 -20 q -4 -10 0 -20" stroke="#d8c9a8" strokeWidth="3" fill="none" strokeLinecap="round" />
            </g>
            {/* owner at door (dawn+) */}
            <circle className={styles.owner} cx="80" cy="80" r="7" fill="#2a1b13" />
          </g>

          {/* Bakery */}
          <g className={`${styles.shop} ${styles.targetShop} ${styles.target1}`} transform="translate(540, 340)">
            <path d="M 0 40 L 60 4 L 120 40 Z" fill="#b4a070" />
            <rect x="8" y="40" width="104" height="80" fill="#d8c9a8" stroke="#9c8757" strokeWidth="2" />
            <rect x="48" y="68" width="24" height="52" fill="#6e5c3f" rx="2" />
            <rect x="20" y="56" width="22" height="22" fill="#fce49a" opacity="0.85" />
            <rect x="78" y="56" width="22" height="22" fill="#fce49a" opacity="0.85" />
            <rect x="20" y="20" width="80" height="16" fill="#8a7751" rx="3" />
            <text x="60" y="32" textAnchor="middle" fontSize="10" fontWeight="700" fill="#fef6e0" fontFamily="system-ui, sans-serif">
              BAKERY
            </text>
            <circle className={styles.targetDot} cx="60" cy="-6" r="5" fill="#d8c9a8" stroke="#9c8757" strokeWidth="1.5" />
          </g>

          {/* Produce */}
          <g className={`${styles.shop} ${styles.targetShop} ${styles.target2}`} transform="translate(770, 340)">
            <path d="M 0 40 L 60 4 L 120 40 Z" fill="#7a8f4c" />
            <rect x="8" y="40" width="104" height="80" fill="#a8c468" stroke="#5e7138" strokeWidth="2" />
            <rect x="48" y="68" width="24" height="52" fill="#3c4a1f" rx="2" />
            <rect x="20" y="56" width="22" height="22" fill="#fce49a" opacity="0.85" />
            <rect x="78" y="56" width="22" height="22" fill="#fce49a" opacity="0.85" />
            <rect x="20" y="20" width="80" height="16" fill="#5e7138" rx="3" />
            <text x="60" y="32" textAnchor="middle" fontSize="10" fontWeight="700" fill="#fef6e0" fontFamily="system-ui, sans-serif">
              PRODUCE
            </text>
            <circle className={styles.targetDot} cx="60" cy="-6" r="5" fill="#a8c468" stroke="#5e7138" strokeWidth="1.5" />
          </g>

          {/* Coffee mill */}
          <g className={`${styles.shop} ${styles.targetShop} ${styles.target3}`} transform="translate(980, 340)">
            <path d="M 0 40 L 60 4 L 120 40 Z" fill="#3a2f28" />
            <rect x="8" y="40" width="104" height="80" fill="#5c4c3e" stroke="#2a221b" strokeWidth="2" />
            <rect x="48" y="68" width="24" height="52" fill="#1a1410" rx="2" />
            <rect x="20" y="56" width="22" height="22" fill="#fce49a" opacity="0.85" />
            <rect x="78" y="56" width="22" height="22" fill="#fce49a" opacity="0.85" />
            <rect x="20" y="20" width="80" height="16" fill="#1a1410" rx="3" />
            <text x="60" y="32" textAnchor="middle" fontSize="10" fontWeight="700" fill="#fef6e0" fontFamily="system-ui, sans-serif">
              MILL
            </text>
            <circle className={styles.targetDot} cx="60" cy="-6" r="5" fill="#5c4c3e" stroke="#1a1410" strokeWidth="1.5" />
          </g>

          {/* Clock */}
          <g className={styles.clock} transform="translate(1080, 450)">
            <rect x="-60" y="-20" width="120" height="40" rx="8" fill="rgba(26,20,16,0.75)" />
            <text x="0" y="6" textAnchor="middle" fontSize="18" fontWeight="700" fill="#fef6e0" fontFamily="ui-monospace, SFMono-Regular, Menlo, monospace">
              <tspan className={styles.time3am}>3:00 AM</tspan>
              <tspan className={styles.time7am} x="0">7:00 AM</tspan>
            </text>
          </g>

          {/* Caption */}
          <text
            x="60"
            y="60"
            fontFamily="var(--font-display, system-ui)"
            fontSize="22"
            fontWeight="700"
            fill="#fef6e0"
            opacity="0.75"
            className={styles.caption}
          >
            A Tuesday night job.
          </text>
        </svg>
      </div>
    </section>
  );
}
