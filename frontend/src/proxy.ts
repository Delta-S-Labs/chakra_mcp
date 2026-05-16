/**
 * Proxy - gate the /app/* routes behind a session.
 *
 * Public auth pages: /login, /signup. Anyone hitting an unauthenticated
 * /app/* URL gets bounced to /login.
 *
 * Logged-in users hitting /login or /signup: we used to bounce them to
 * /app, which felt like "auto-login" when really they just had a stale
 * session cookie and wanted to switch accounts. Now we let those pages
 * render and they show a "Signed in as X — sign out to switch?" banner
 * up top. The user picks: stay signed in (Continue link), or sign out
 * and pick a different identity. Much less confusing.
 *
 * NextAuth.js v5 exposes `auth` as middleware directly.
 */

import { NextResponse } from "next/server";
import { auth } from "@/auth";

const PUBLIC_AUTH_PATHS = new Set(["/login", "/signup"]);
const isInvitePath = (pathname: string) => pathname.startsWith("/invites/");
const isOAuthPath = (pathname: string) => pathname.startsWith("/oauth/");

export default auth((req) => {
  const { pathname, search } = req.nextUrl;
  // /invites/<token> renders for both signed-in and signed-out users; the
  // page itself decides what to render. Always pass through.
  if (isInvitePath(pathname)) {
    return NextResponse.next();
  }
  const isLoggedIn = !!req.auth;

  // OAuth consent: if signed out, send to /login with the original
  // /oauth/authorize?... preserved as `from`. The login page already
  // honours `from` and bounces back after sign-in.
  if (isOAuthPath(pathname)) {
    if (!isLoggedIn) {
      const url = req.nextUrl.clone();
      url.pathname = "/login";
      url.searchParams.set("from", `${pathname}${search}`);
      return NextResponse.redirect(url);
    }
    return NextResponse.next();
  }

  const isPublicAuth = PUBLIC_AUTH_PATHS.has(pathname);

  if (!isLoggedIn && !isPublicAuth) {
    const url = req.nextUrl.clone();
    url.pathname = "/login";
    // Include the original query string in `from`. A QR-scan device-flow
    // pairing lands the user at `/app/pair?session=ABCD-1234` — if we
    // strip `?session=...` here, after sign-in they get bounced to a
    // bare `/app/pair` (the code-entry landing page) instead of the
    // consent screen. The CLI keeps polling the unapproved code until
    // it expires and the user reasonably thinks "the pairing did not
    // happen." The OAuth branch above (line ~40) already does this
    // — same fix, same shape.
    url.searchParams.set("from", `${pathname}${search}`);
    return NextResponse.redirect(url);
  }

  // Intentionally allow /login + /signup through even when signed in.
  // The pages themselves show a "Signed in as X" banner so the user
  // can switch accounts without feeling auto-logged-in.
  return NextResponse.next();
});

// Run only on the relay-app surfaces and the public auth pages. The
// marketing site (/, /concept, /brand, /cofounder, /terms) and render
// targets stay public.
export const config = {
  matcher: [
    "/app",
    "/app/:path*",
    "/login",
    "/signup",
    "/invites/:path*",
    "/oauth/:path*",
  ],
};
