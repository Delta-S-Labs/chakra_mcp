/**
 * Proxy — gate the /app/* routes behind a session.
 *
 * Public auth pages: /login, /signup. Anyone hitting an unauthenticated
 * /app/* URL gets bounced to /login. Logged-in users hitting /login or
 * /signup get bounced to /app.
 *
 * NextAuth.js v5 exposes `auth` as middleware directly.
 */

import { NextResponse } from "next/server";
import { auth } from "@/auth";

const PUBLIC_AUTH_PATHS = new Set(["/login", "/signup"]);
const isInvitePath = (pathname: string) => pathname.startsWith("/invites/");

export default auth((req) => {
  const { pathname } = req.nextUrl;
  // /invites/<token> renders for both signed-in and signed-out users; the
  // page itself decides what to render. Always pass through.
  if (isInvitePath(pathname)) {
    return NextResponse.next();
  }
  const isPublicAuth = PUBLIC_AUTH_PATHS.has(pathname);
  const isLoggedIn = !!req.auth;

  if (!isLoggedIn && !isPublicAuth) {
    const url = req.nextUrl.clone();
    url.pathname = "/login";
    url.searchParams.set("from", pathname);
    return NextResponse.redirect(url);
  }

  if (isLoggedIn && isPublicAuth) {
    const url = req.nextUrl.clone();
    url.pathname = "/app";
    return NextResponse.redirect(url);
  }

  return NextResponse.next();
});

// Run only on the relay-app surfaces and the public auth pages. The
// marketing site (/, /concept, /brand, /cofounder, /terms) and render
// targets stay public.
export const config = {
  matcher: ["/app", "/app/:path*", "/login", "/signup", "/invites/:path*"],
};
