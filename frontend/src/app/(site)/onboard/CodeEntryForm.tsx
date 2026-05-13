"use client";

import { useState } from "react";
import { useRouter } from "next/navigation";
import styles from "./onboard.module.css";

/**
 * "Enter your pairing code" form — the TV-style manual entry path.
 *
 * The agent printed something like `ABCD-1234`; the user types it here
 * and we redirect to /onboard?session=<canonical>. The consent screen
 * takes over from there.
 */
export function CodeEntryForm() {
  const router = useRouter();
  const [code, setCode] = useState("");
  const [error, setError] = useState<string | null>(null);

  function handleSubmit(e: React.FormEvent) {
    e.preventDefault();
    setError(null);
    const cleaned = code.toUpperCase().replace(/[\s-]+/g, "");
    if (!/^[A-Z0-9]{8}$/.test(cleaned)) {
      setError(
        "Pairing codes are 8 characters of A-Z / 0-9 (e.g. ABCD-1234). " +
          `Got ${cleaned.length} character${cleaned.length === 1 ? "" : "s"}.`,
      );
      return;
    }
    const canonical = `${cleaned.slice(0, 4)}-${cleaned.slice(4)}`;
    router.push(`/onboard?session=${canonical}`);
  }

  return (
    <form onSubmit={handleSubmit}>
      <div className={styles.codeRow}>
        <input
          type="text"
          className={styles.codeInput}
          placeholder="ABCD-1234"
          maxLength={9}
          autoComplete="off"
          autoCapitalize="characters"
          autoCorrect="off"
          spellCheck={false}
          inputMode="text"
          value={code}
          onChange={(e) => setCode(e.target.value)}
          aria-label="Pairing code"
        />
        <button type="submit" className={styles.btnPrimary}>
          Continue
        </button>
      </div>
      {error && <div className={styles.error}>{error}</div>}
    </form>
  );
}
