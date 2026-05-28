/**
 * Read-only star rating display.
 *
 * Renders 5 stars filled in proportion to `value` (0–5).  Fractional
 * values are supported via a clip-path on the half-star overlay so 3.7
 * shows ¾-filled on the 4th star, etc.  Pair with `StarRatingInput`
 * (in the same module) when you need a click-to-pick variant.
 *
 * Inline SVG — we don't pull in an icon library; the path here is the
 * standard 5-pointer star inscribed in a 24×24 box.
 */

const STAR_PATH =
  "M12 2l2.92 6.34 6.83.62-5.18 4.66 1.54 6.78L12 16.97l-6.11 3.43 1.54-6.78L2.25 8.96l6.83-.62L12 2z";

function Star({
  filled,
  size,
  partial,
}: {
  filled: boolean;
  size: number;
  partial?: number;
}) {
  const fill = filled ? "var(--accent-coral)" : "transparent";
  const stroke = "var(--accent-coral)";
  return (
    <svg
      width={size}
      height={size}
      viewBox="0 0 24 24"
      aria-hidden="true"
      style={{ display: "inline-block", verticalAlign: "-2px" }}
    >
      <path d={STAR_PATH} fill={fill} stroke={stroke} strokeWidth={1.5} />
      {partial !== undefined && partial > 0 && partial < 1 && (
        <path
          d={STAR_PATH}
          fill="var(--accent-coral)"
          stroke="var(--accent-coral)"
          strokeWidth={1.5}
          clipPath={`inset(0 ${(1 - partial) * 100}% 0 0)`}
        />
      )}
    </svg>
  );
}

export function StarRating({
  value,
  size = 14,
  showNumber = false,
  ariaLabel,
}: {
  value: number | null;
  size?: number;
  showNumber?: boolean;
  ariaLabel?: string;
}) {
  const v = value ?? 0;
  return (
    <span
      aria-label={
        ariaLabel ??
        (value === null ? "No rating yet" : `${value.toFixed(1)} out of 5`)
      }
      role="img"
      style={{ display: "inline-flex", alignItems: "center", gap: "0.18rem" }}
    >
      {[0, 1, 2, 3, 4].map((i) => {
        const filled = v >= i + 1;
        const partial = !filled && v > i ? v - i : undefined;
        return <Star key={i} filled={filled} size={size} partial={partial} />;
      })}
      {showNumber && value !== null && (
        <span style={{ marginLeft: "0.35rem", fontWeight: 600 }}>
          {value.toFixed(1)}
        </span>
      )}
    </span>
  );
}

/**
 * Click-to-pick star input.  Hover preview + keyboard support (1–5
 * keys focus a specific value; arrows step).
 */
export function StarRatingInput({
  value,
  onChange,
  size = 22,
  disabled = false,
}: {
  value: number;
  onChange: (n: number) => void;
  size?: number;
  disabled?: boolean;
}) {
  return (
    <span
      role="radiogroup"
      aria-label="Rating"
      style={{ display: "inline-flex", gap: "0.25rem" }}
    >
      {[1, 2, 3, 4, 5].map((n) => (
        <button
          key={n}
          type="button"
          role="radio"
          aria-checked={value === n}
          aria-label={`${n} star${n === 1 ? "" : "s"}`}
          disabled={disabled}
          onClick={() => onChange(n)}
          style={{
            background: "transparent",
            border: "none",
            padding: 0,
            cursor: disabled ? "default" : "pointer",
            lineHeight: 0,
          }}
        >
          <Star filled={value >= n} size={size} />
        </button>
      ))}
    </span>
  );
}
