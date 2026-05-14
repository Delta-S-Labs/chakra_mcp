/**
 * Auth server actions — sign out is the only one that needs sharing.
 *
 * Why a shared helper? The sign-out flow is hard to get right under
 * NextAuth v5 beta + Next.js 16, and we have two call sites today (the
 * app shell layout and the `<AlreadySignedIn />` banner on /login). We
 * burned a P0 on a partial fix once; now there is one implementation
 * and both screens import it.
 *
 * # The bug we are working around
 *
 * NextAuth v5 (`5.0.0-beta.31`) in JWT-session mode stores the
 * session inside a single cookie: `authjs.session-token` (or
 * `__Secure-authjs.session-token` on HTTPS). Calling `signOut()`
 * *should* clear that cookie, and on the next request the proxy in
 * `src/proxy.ts` should see `req.auth === null` and bounce the user
 * to `/login`.
 *
 * In practice, the previous mitigation —
 *
 *     await signOut({ redirect: false });
 *     revalidatePath("/", "layout");
 *     redirect("/login");
 *
 * — is not enough. The failure mode is: click "Sign out", land on
 * `/login`, then hit Back or paste `/app` in the address bar, and
 * the app shell renders fully authenticated. No bounce. The session
 * cookie is still alive.
 *
 * Two things are going wrong, both real, both subtle:
 *
 *   1. `signOut()` in `@auth/core` only walks the `SessionStore`
 *      (see `node_modules/@auth/core/src/lib/utils/cookie.ts` →
 *      `SessionStore.clean()`) and emits cleaning `Set-Cookie`
 *      headers for cookies that match the configured
 *      `sessionToken.name` prefix AND were present on the inbound
 *      `Cookie:` header it parsed. When the action runs through
 *      Netlify's edge wrapper + Next.js 16's action runtime, the
 *      request that `signOut()` synthesizes internally does not
 *      always inherit those cookies, so `SessionStore` is empty and
 *      `clean()` is a no-op — signOut silently does nothing.
 *
 *   2. Even when (1) works, the cleaning `Set-Cookie` headers are
 *      attached to the *server-action* response. The action then
 *      throws `NEXT_REDIRECT`, and Next 16 composes a separate
 *      redirect response. `Set-Cookie` is supposed to be preserved
 *      across that composition, but in NextAuth v5 beta on Netlify
 *      we have observed cases where it is not — the redirect ships
 *      without the cookie-deleting headers and the browser keeps
 *      the old cookie.
 *
 * # The fix
 *
 * Belt and braces:
 *
 *   - Still call `signOut({ redirect: false })` so any side-effects
 *     wired into NextAuth's `events.signOut` callback fire.
 *   - Then explicitly `cookies().delete(...)` every NextAuth cookie
 *     name we know about, in both the unprefixed (HTTP dev) and
 *     `__Secure-` / `__Host-` (HTTPS prod) forms. `cookies().delete`
 *     goes through Next's own outgoing-headers buffer, which is
 *     plumbed into the action response in a way that *does* survive
 *     the redirect composition.
 *   - Skip `revalidatePath`. It does not clear cookies, and calling
 *     it between cookie writes and `redirect()` has been reported as
 *     a race trigger in the Next 16 action runtime. The redirect to
 *     `/login` lands on a different page so there is no stale RSC
 *     fragment of the authed shell to bust.
 *
 * # Backend session invalidation — NOT done here
 *
 * The backend (`backend/app/src/handlers/auth.rs`) issues stateless
 * JWTs with no server-side session table and no revocation endpoint.
 * The JWT lives inside the NextAuth cookie payload, so clearing the
 * cookie hides it from the user agent — which is the full extent of
 * what the client can do today. A stolen / exfiltrated JWT remains
 * valid until its 24h `exp`. Adding a revocation list (or short
 * access tokens + refresh tokens) is tracked as a separate backend
 * slice; this file cannot fix it.
 */

"use server";

import { cookies } from "next/headers";
import { redirect } from "next/navigation";

import { signOut } from "@/auth";

/**
 * Every cookie NextAuth v5 may have set. We try to delete all of
 * them — `cookies().delete()` is a no-op when the cookie is not
 * present, so the spray is cheap.
 *
 * Cookies with `__Secure-` or `__Host-` prefixes have RFC 6265bis
 * rules: the deleting `Set-Cookie` MUST itself carry the `Secure`
 * flag (and `__Host-` requires `path=/` with no `domain`) or the
 * browser silently ignores the deletion. We encode those constraints
 * here so the loop just spreads `options` into `cookies().delete()`.
 *
 * Source of truth: `@auth/core/src/lib/utils/cookie.ts` →
 * `defaultCookies()`. Mirrored here because we cannot import from
 * that internal path without breaking on every NextAuth bump.
 */
type CookieDeleteOptions = {
  name: string;
  path: string;
  secure?: boolean;
};

const NEXT_AUTH_COOKIES: readonly CookieDeleteOptions[] = [
  // Session JWT — the one that actually authenticates.
  { name: "authjs.session-token", path: "/" },
  { name: "__Secure-authjs.session-token", path: "/", secure: true },
  // Callback URL — not authenticating, but a stale "where to go
  // after login" surprises users on the next sign-in.
  { name: "authjs.callback-url", path: "/" },
  { name: "__Secure-authjs.callback-url", path: "/", secure: true },
  // CSRF — should not authenticate on its own, but clearing it
  // forces a fresh CSRF round-trip on the next sign-in. The
  // production cookie uses the `__Host-` prefix (stricter than
  // `__Secure-`) per NextAuth defaults.
  { name: "authjs.csrf-token", path: "/" },
  { name: "__Host-authjs.csrf-token", path: "/", secure: true },
  // OAuth handshake cookies. Usually short-lived (maxAge 15min) but
  // clear them anyway so a half-completed handshake does not leak
  // into the next session.
  { name: "authjs.pkce.code_verifier", path: "/" },
  { name: "__Secure-authjs.pkce.code_verifier", path: "/", secure: true },
  { name: "authjs.state", path: "/" },
  { name: "__Secure-authjs.state", path: "/", secure: true },
  { name: "authjs.nonce", path: "/" },
  { name: "__Secure-authjs.nonce", path: "/", secure: true },
] as const;

/**
 * Sign the user out and bounce to `/login`.
 *
 * Use this from any server-action `action={signOutAndRedirect}` form
 * inside the `(app)` route group. Do **not** call `signOut()`
 * directly from a form action without this wrapper — see the file
 * header for why.
 */
export async function signOutAndRedirect(): Promise<never> {
  // Run NextAuth's own signOut so any wired-up `events.signOut` hook
  // fires. We pass `redirect: false` because we own the redirect
  // ourselves, after the manual cookie purge below.
  try {
    await signOut({ redirect: false });
  } catch (err) {
    // NextAuth v5 beta has been observed to throw an internal error
    // here in rare edge-runtime conditions. We do not want a failed
    // event hook to prevent the cookie purge — that is the part that
    // actually logs the user out.
    console.error(
      "[auth] signOut() threw, falling through to manual cookie purge",
      err,
    );
  }

  const cookieStore = await cookies();
  for (const opts of NEXT_AUTH_COOKIES) {
    // `delete` is a no-op if the cookie is not in the store. Pass
    // the options form so `path`, and (for `__Secure-` / `__Host-`
    // prefixed cookies) `secure`, match what NextAuth set — browsers
    // silently ignore deletions whose path or prefix-required
    // attributes do not match.
    cookieStore.delete(opts);
  }

  redirect("/login");
}
