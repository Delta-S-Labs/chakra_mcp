"use client";

import { useEffect, useRef, useState } from "react";
import { usePathname, useRouter } from "next/navigation";
import styles from "./commandPalette.module.css";

/**
 * Global Cmd+K (Ctrl+K on non-Mac) command palette + vim-style `g <key>`
 * navigation chords.
 *
 * Mounted once from `app/(app)/app/layout.tsx`, so the listener is alive
 * everywhere under `/app/*`.
 *
 * Behaviour:
 *   - Cmd/Ctrl+K toggles a centered modal listing every binding.
 *   - ESC closes the modal.
 *   - Click outside the panel closes the modal.
 *   - Typing `g` then within 1.5s a second key triggers navigation
 *     (works whether the palette is open or closed).
 *   - The chord listener is suppressed when the focused element is an
 *     <input>, <textarea>, <select>, or contenteditable — typing the
 *     literal letter "g" in a form must not steal the keystroke.
 *
 * The chord state lives in a `useRef`, not `useState`: re-renders in
 * the middle of the chord (e.g. someone hovers an item between `g`
 * and `a`) would otherwise reset `prefix` and break the chord.
 */

type Command = {
  /** Display label, e.g. "Agents". */
  label: string;
  /** Route to push, e.g. "/app/agents". */
  href: string;
  /** Second key of the `g <key>` chord, e.g. "a". */
  key: string;
  /** True when `pathname === href`; for nav with sub-routes use a startsWith check. */
  exact?: boolean;
};

const COMMANDS: Command[] = [
  { label: "Dashboard", href: "/app", key: "d", exact: true },
  { label: "Agents", href: "/app/agents", key: "a" },
  { label: "Network agents", href: "/app/agents/network", key: "n" },
  { label: "Friendships", href: "/app/friendships", key: "f" },
  { label: "Grants", href: "/app/grants", key: "g" },
  { label: "Inbox", href: "/app/inbox", key: "i" },
  { label: "Usage", href: "/app/usage", key: "u" },
  { label: "Audit", href: "/app/audit", key: "x" },
  { label: "API keys", href: "/app/api-keys", key: "k" },
  { label: "Pair agent", href: "/app/pair", key: "p" },
  { label: "Orgs", href: "/app/orgs", key: "o" },
];

const CHORD_TIMEOUT_MS = 1500;

function isEditableTarget(el: Element | null): boolean {
  if (!el) return false;
  const tag = el.tagName;
  if (tag === "INPUT" || tag === "TEXTAREA" || tag === "SELECT") return true;
  if ((el as HTMLElement).isContentEditable) return true;
  return false;
}

function detectMac(): boolean {
  if (typeof navigator === "undefined") return false;
  return /Mac|iPhone|iPad|iPod/i.test(navigator.platform || navigator.userAgent);
}

export function CommandPalette() {
  const router = useRouter();
  const pathname = usePathname();
  const [open, setOpen] = useState(false);

  // Mac vs. non-Mac drives only the displayed modifier glyph (⌘ vs Ctrl).
  // Lazy-init via `useState(fn)` runs the initializer exactly once per
  // mount on the client, so we never read `navigator` during SSR.
  const [mac] = useState(() => detectMac());

  // Chord state — see file-level docstring for why this is a ref, not state.
  const chordRef = useRef<{
    prefix: string | null;
    timer: ReturnType<typeof setTimeout> | null;
  }>({ prefix: null, timer: null });

  useEffect(() => {
    function clearChord() {
      if (chordRef.current.timer) {
        clearTimeout(chordRef.current.timer);
      }
      chordRef.current = { prefix: null, timer: null };
    }

    function navigate(href: string) {
      clearChord();
      setOpen(false);
      router.push(href);
    }

    function onKeyDown(e: KeyboardEvent) {
      // Cmd+K / Ctrl+K — works even when an input is focused (palette
      // is the standard exception to the editable-target rule).
      if ((e.metaKey || e.ctrlKey) && !e.shiftKey && !e.altKey && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setOpen((v) => !v);
        clearChord();
        return;
      }

      if (e.key === "Escape") {
        if (open) {
          e.preventDefault();
          setOpen(false);
        }
        clearChord();
        return;
      }

      // Don't run chord logic while the user is typing in a form
      // element. The palette itself has no form inputs, so this also
      // intentionally suppresses chords when (eg) a textarea elsewhere
      // on the page is focused but the palette is closed.
      if (isEditableTarget(document.activeElement)) {
        return;
      }

      // Ignore any chord that comes with a modifier — leaves
      // Cmd/Ctrl/Alt shortcuts (and OS-level ones) untouched.
      if (e.metaKey || e.ctrlKey || e.altKey) {
        return;
      }

      const key = e.key.toLowerCase();
      // Only single printable letter keys participate in the chord.
      if (key.length !== 1 || !/[a-z]/.test(key)) {
        return;
      }

      if (chordRef.current.prefix === "g") {
        const match = COMMANDS.find((c) => c.key === key);
        clearChord();
        if (match) {
          e.preventDefault();
          navigate(match.href);
        }
        return;
      }

      if (key === "g") {
        clearChord();
        chordRef.current.prefix = "g";
        chordRef.current.timer = setTimeout(clearChord, CHORD_TIMEOUT_MS);
      }
    }

    window.addEventListener("keydown", onKeyDown);
    return () => {
      window.removeEventListener("keydown", onKeyDown);
      clearChord();
    };
  }, [open, router]);

  if (!open) return null;

  const isCurrent = (c: Command) =>
    c.exact ? pathname === c.href : pathname === c.href || pathname.startsWith(`${c.href}/`);

  const modKey = mac ? "⌘" : "Ctrl";

  return (
    <div
      className={styles.backdrop}
      role="dialog"
      aria-modal="true"
      aria-label="Command palette"
      onMouseDown={(e) => {
        // Close on backdrop click only — clicks inside the panel are
        // filtered by the inner stopPropagation below.
        if (e.target === e.currentTarget) {
          setOpen(false);
        }
      }}
    >
      <div className={styles.panel} onMouseDown={(e) => e.stopPropagation()}>
        <div className={styles.header}>
          <h2 className={styles.title}>Jump to</h2>
          <span className={styles.hint}>
            <span className={styles.key}>{modKey}</span>
            <span className={styles.hintPlus}>+</span>
            <span className={styles.key}>K</span>
            <span className={styles.hintSep}>to toggle &middot; Esc to close</span>
          </span>
        </div>

        <ul className={styles.list} role="listbox">
          {COMMANDS.map((c) => (
            <li key={c.href}>
              <button
                type="button"
                className={`${styles.item} ${isCurrent(c) ? styles.itemActive : ""}`}
                onClick={() => {
                  setOpen(false);
                  router.push(c.href);
                }}
                aria-current={isCurrent(c) ? "page" : undefined}
              >
                <span className={styles.label}>{c.label}</span>
                <span className={styles.chord} aria-label={`Shortcut: g then ${c.key}`}>
                  <span className={styles.key}>g</span>
                  <span className={styles.key}>{c.key}</span>
                </span>
              </button>
            </li>
          ))}
        </ul>
      </div>
    </div>
  );
}
