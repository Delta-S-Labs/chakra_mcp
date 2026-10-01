/**
 * Where to send someone after they sign in or sign up.
 *
 * The target arrives in the URL (`/login?from=…`), so anyone can craft
 * it. Only a path on this site is allowed:
 *
 *   - it starts with one `/`. `//host` and `/\host` mean another site to
 *     a browser.
 *   - it has no backslashes or control characters, raw or percent-decoded.
 *     Browsers turn `\` into `/` and drop tabs and newlines, so
 *     `/\t/host` also leaves the site.
 *   - it still resolves to this origin.
 *
 * Anything else falls back to `/app`, including absolute URLs (even to
 * this site), `javascript:` URLs and repeated query parameters. The
 * NextAuth cookie that a sign-in sets carries the backend token, so a
 * script run after sign-in could read it.
 */

export const DEFAULT_REDIRECT = "/app";

// Never a real origin: only used to resolve the path.
const BASE = "https://redirect-check.invalid";

// Backslash, the C0 controls, and DEL.
const UNSAFE_CHARS = /[\\\u0000-\u001f\u007f]/;

export function safeRedirect(target: unknown, fallback: string = DEFAULT_REDIRECT): string {
  if (typeof target !== "string") return fallback;
  if (!isSafePath(target)) return fallback;

  let decoded: string;
  try {
    decoded = decodeURIComponent(target);
  } catch {
    return fallback;
  }
  if (!isSafePath(decoded)) return fallback;

  let url: URL;
  try {
    url = new URL(target, BASE);
  } catch {
    return fallback;
  }
  if (url.origin !== BASE) return fallback;
  return `${url.pathname}${url.search}${url.hash}`;
}

function isSafePath(path: string): boolean {
  return path.startsWith("/") && !path.startsWith("//") && !UNSAFE_CHARS.test(path);
}
