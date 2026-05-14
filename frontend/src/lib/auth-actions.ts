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
 * # Backend JWT revocation
 *
 * The backend (`backend/app/src/handlers/auth.rs`) issues stateless
 * 24h JWTs. Clearing the cookie hides the JWT from the user agent
 * but doesn't kill the token itself — anyone who exfiltrated the
 * plaintext can keep using it until natural expiry.
 *
 * We close that hole by POSTing `/v1/auth/signout` *before* the
 * cookie purge, while we still have the Bearer. The backend records
 * the `jti` in its `revoked_tokens` table and the JWT-decode
 * middleware rejects any further request that carries it.
 *
 * The backend call is best-effort: if it fails (network down, server
 * unreachable, or the token was already revoked and the call 401s),
 * we still continue with the cookie purge + redirect. The local
 * logout always works, even when the server is gone.
 */

"use server";

import { cookies } from "next/headers";
import { redirect } from "next/navigation";

import { auth, signOut } from "@/auth";
import { apiBaseUrl } from "@/lib/api";

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
  // Server-side JWT revocation FIRST, while we still have the
  // Bearer. Clearing the cookie hides the token from this browser
  // but doesn't kill it; only the backend can do that. We swallow
  // any failure: network errors, 401 (token already revoked), or a
  // backend outage must not block the local logout.
  try {
    const session = await auth();
    const backendToken = session?.backendToken;
    if (backendToken) {
      const res = await fetch(`${apiBaseUrl}/v1/auth/signout`, {
        method: "POST",
        headers: { authorization: `Bearer ${backendToken}` },
        cache: "no-store",
      });
      if (!res.ok && res.status !== 401) {
        // 401 is expected if the token is already revoked (e.g. the
        // user double-clicked sign out). Anything else gets logged
        // but does not abort the cookie purge.
        console.warn(
          `[auth] backend signout returned ${res.status}; proceeding with cookie purge`,
        );
      }
    }
  } catch (err) {
    console.warn(
      "[auth] backend signout failed; proceeding with cookie purge",
      err,
    );
  }

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
