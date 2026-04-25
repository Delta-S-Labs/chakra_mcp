/**
 * NextAuth.js v5 — relay web app authentication.
 *
 * Two providers wired up: GitHub and Google. Credentials live in
 * `.env.local` (gitignored). NextAuth derives AUTH_SECRET in dev if
 * unset, but we always set NEXTAUTH_SECRET for consistency.
 *
 * Captcha verification is handled separately in
 * src/app/api/captcha/verify/route.ts and gated by CAPTCHA_ENABLED.
 */

import NextAuth from "next-auth";
import GitHub from "next-auth/providers/github";
import Google from "next-auth/providers/google";

export const { handlers, signIn, signOut, auth } = NextAuth({
  // NextAuth v5 reads AUTH_SECRET by default; we also accept NEXTAUTH_SECRET
  // for parity with v4 .env files and our .env.example.
  secret: process.env.AUTH_SECRET ?? process.env.NEXTAUTH_SECRET,
  providers: [
    GitHub({
      clientId: process.env.GITHUB_CLIENT_ID,
      clientSecret: process.env.GITHUB_CLIENT_SECRET,
    }),
    Google({
      clientId: process.env.GOOGLE_CLIENT_ID,
      clientSecret: process.env.GOOGLE_CLIENT_SECRET,
    }),
  ],
  pages: {
    signIn: "/login",
  },
  session: { strategy: "jwt" },
  trustHost: true,
});
