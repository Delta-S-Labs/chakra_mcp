/**
 * Format-helpers shared across the app shell.
 *
 * Keep these tiny and well-tested — they show up in tight UI loops
 * (audit log rows, dashboard cards) where a wrong format throws off
 * the at-a-glance read.
 */

/**
 * Render a duration in milliseconds as something a human can scan in
 * one beat. Rules, in priority order:
 *
 *   ms < 1000             → `"<N>ms"`       e.g. 250ms     (sub-second:
 *                                                            keep ms;
 *                                                            seconds
 *                                                            would round
 *                                                            to 0.)
 *   ms < 10_000           → `"<N.N>s"`      e.g. 4.9s      (single
 *                                                            decimal under
 *                                                            10s so we
 *                                                            don't lose
 *                                                            sub-second
 *                                                            precision.)
 *   ms < 60_000           → `"<N>s"`        e.g. 14s
 *   ms < 60 * 60_000      → `"<M>m <S>s"`   e.g. 1m 1s,
 *                                            2m 30s
 *   ms ≥ 60 * 60_000      → `"<H>h <M>m"`   e.g. 1h 5m
 *
 * Negative or non-finite inputs collapse to `"—"`. Zero collapses to
 * `"—"` too so empty/un-instrumented rows don't draw the eye to a
 * meaningless "0ms".
 */
export function formatElapsed(ms: number): string {
  if (!Number.isFinite(ms) || ms <= 0) return "—";

  if (ms < 1000) return `${Math.round(ms)}ms`;

  const totalSeconds = ms / 1000;

  if (totalSeconds < 10) {
    // 1.0s–9.9s — single decimal, so "4858ms" reads as "4.9s" not "5s".
    return `${totalSeconds.toFixed(1)}s`;
  }

  if (totalSeconds < 60) {
    return `${Math.round(totalSeconds)}s`;
  }

  if (totalSeconds < 60 * 60) {
    const m = Math.floor(totalSeconds / 60);
    const s = Math.round(totalSeconds - m * 60);
    return s === 0 ? `${m}m` : `${m}m ${s}s`;
  }

  const h = Math.floor(totalSeconds / 3600);
  const m = Math.round((totalSeconds - h * 3600) / 60);
  return m === 0 ? `${h}h` : `${h}h ${m}m`;
}
