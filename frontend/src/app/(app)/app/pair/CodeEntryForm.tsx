"use client";

import { useMemo, useState } from "react";
import { useRouter } from "next/navigation";
import styles from "./pair.module.css";

/**
 * "Enter your pairing code" form — the TV-style manual entry path.
 *
 * The agent printed something like `ABCD-1234`; the user types it here
 * and we redirect to /app/pair?session=<canonical>. The consent screen
 * takes over from there.
 *
 * # Input liberality
 *
 * The canonical wire shape is 8 characters of `[A-Z0-9]` plus a hyphen
 * at position 4 (e.g. `ABCD-1234`). Users type it however they want:
 *
 *   * `ABCD-1234` (canonical)
 *   * `abcd-1234` (lowercase — phones auto-lowercase)
 *   * `ABCD1234`  (no hyphen — reported live: users assumed they had
 *                  to strip the dash because the old `maxLength={9}`
 *                  felt like "exactly 8 characters")
 *   * `ABCD–1234` (en-dash / em-dash — happens via copy from rich text)
 *   * `ABCD 1234` (space instead of dash — also seen in pasted output)
 *   * `  ABCD-1234  ` (leading/trailing whitespace from clipboard)
 *
 * We strip every non-`[A-Z0-9]` character before validating, and
 * `maxLength` is set generously (20) so pasted "ugly" forms don't get
 * truncated before we get a chance to clean them. The live preview
 * under the input tells the user exactly what canonical code we'll
 * submit, so they're never guessing whether their input survived.
 */

/** Strip any non-alphanumeric char and uppercase. Empty string ⇒ no chars. */
function normalizeRawInput(raw: string): string {
  return raw.toUpperCase().replace(/[^A-Z0-9]+/g, "");
}

/** Canonical wire form: `ABCD-1234`. Returns null if input isn't usable. */
function toCanonical(raw: string): string | null {
  const cleaned = normalizeRawInput(raw);
  if (!/^[A-Z0-9]{8}$/.test(cleaned)) return null;
  return `${cleaned.slice(0, 4)}-${cleaned.slice(4)}`;
}

export function CodeEntryForm() {
  const router = useRouter();
  const [code, setCode] = useState("");
  const [error, setError] = useState<string | null>(null);

  // Live preview of the canonical form we'll submit. Helps the user
  // see that "abcd1234" and "ABCD-1234" both resolve to the same code,
  // and gives feedback before they hit Continue.
  const cleaned = useMemo(() => normalizeRawInput(code), [code]);
  const canonical = useMemo(() => toCanonical(code), [code]);

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    if (canonical) {
      router.push(`/app/pair?session=${canonical}`);
      return;
    }
    if (cleaned.length === 0) {
      setError("Type the pairing code your agent printed (e.g. ABCD-1234).");
      return;
    }
    setError(
      `Pairing codes are 8 letters or digits (we saw ${cleaned.length}: ` +
        `\`${cleaned}\`). Hyphens, spaces, and case don't matter — both ` +
        "`ABCD-1234` and `abcd1234` work.",
    );
  }

  return (
    <form onSubmit={handleSubmit}>
      <div className={styles.codeRow}>
        <input
          type="text"
          className={styles.codeInput}
          placeholder="ABCD-1234"
          // Generous: lets users paste in "  ABCD - 1234  " and have us
          // clean it. Even 20 chars of garbage will be reduced to 8
          // alphanumerics or rejected as not-a-code.
          maxLength={20}
          autoComplete="off"
          autoCapitalize="characters"
          autoCorrect="off"
          spellCheck={false}
          inputMode="text"
          value={code}
          onChange={(e) => setCode(e.target.value)}
          aria-label="Pairing code"
          aria-describedby="pair-code-hint"
        />
        <button type="submit" className={styles.btnPrimary}>
          Continue
        </button>
      </div>
      <div id="pair-code-hint" className={styles.codeHint}>
        {canonical
          ? `Will submit as ${canonical}`
          : "Hyphen, case, and spaces don’t matter — both ABCD-1234 and abcd1234 work."}
      </div>
      {error && <div className={styles.error}>{error}</div>}
    </form>
  );
}
