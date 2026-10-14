"use client";

import Link from "next/link";
import { useState, useSyncExternalStore } from "react";
import type { AlertBanner } from "@/lib/credit-alerts";
import styles from "./CreditAlertBanner.module.css";

const STORAGE_KEY = "chakramcp.creditBanner.dismissed";
const CHANGED = "chakramcp:credit-banner";

function subscribe(onChange: () => void): () => void {
  window.addEventListener("storage", onChange);
  window.addEventListener(CHANGED, onChange);
  return () => {
    window.removeEventListener("storage", onChange);
    window.removeEventListener(CHANGED, onChange);
  };
}

/** The alerts the viewer dismissed this session, if storage is readable. */
function dismissedKey(): string | null {
  try {
    return sessionStorage.getItem(STORAGE_KEY);
  } catch {
    return null; // Private mode or blocked site data: just show the banner.
  }
}

/** The banner itself. A low-balance banner can be dismissed for this browser
 *  session; it comes back when the alerts change. Out of credits stays. */
export function CreditAlertBannerView({ banner }: { banner: AlertBanner }) {
  const stored = useSyncExternalStore(subscribe, dismissedKey, () => null);
  // Also hides it for this page view when storage can't be written.
  const [dismissedNow, setDismissedNow] = useState(false);
  if (banner.dismissible && (dismissedNow || stored === banner.dismissKey)) return null;

  const blocked = banner.lines.some((l) => l.tone === "blocked");
  return (
    <div
      className={blocked ? styles.blocked : styles.low}
      role={blocked ? "alert" : "status"}
      aria-label="Credits"
    >
      <ul className={styles.lines}>
        {banner.lines.map((line) => (
          <li key={line.key}>
            <span>{line.text}</span>{" "}
            <Link className={styles.action} href={line.href}>
              {banner.action}
            </Link>
          </li>
        ))}
        {banner.more && <li className={styles.more}>{banner.more}</li>}
      </ul>
      {banner.dismissible && (
        <button
          type="button"
          className={styles.dismiss}
          aria-label="Dismiss for now"
          onClick={() => {
            try {
              sessionStorage.setItem(STORAGE_KEY, banner.dismissKey);
              window.dispatchEvent(new Event(CHANGED));
            } catch {
              // Dismissed for this page view only.
            }
            setDismissedNow(true);
          }}
        >
          ×
        </button>
      )}
    </div>
  );
}
