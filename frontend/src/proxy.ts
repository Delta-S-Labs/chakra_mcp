/**
 * Middleware — gate the /app/* routes behind a session.
 *
 * Anyone hitting an unauthenticated /app/* URL gets bounced to /login.
 * /login itself is public so users can actually sign in.
 *
 * NextAuth.js v5 exposes `auth` as a middleware function directly.
 */

import { NextResponse } from "next/server";
import { auth } from "@/auth";

export default auth((req) => {
  const { pathname } = req.nextUrl;
  const isLoginPage = pathname === "/login";
  const isLoggedIn = !!req.auth;

  if (!isLoggedIn && !isLoginPage) {
    const url = req.nextUrl.clone();
    url.pathname = "/login";
    url.searchParams.set("from", pathname);
    return NextResponse.redirect(url);
  }

  if (isLoggedIn && isLoginPage) {
    const url = req.nextUrl.clone();
    url.pathname = "/app";
    return NextResponse.redirect(url);
  }

  return NextResponse.next();
});

// Run only on the relay-app surfaces and the login page. The marketing
// site (/, /concept, /brand, /cofounder) and render targets stay public.
export const config = {
  matcher: ["/app", "/app/:path*", "/login"],
};
