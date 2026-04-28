/**
 * NextAuth.js v5 - relay web app authentication.
 *
 * Two providers wired up: GitHub and Google. Credentials live in
 * `.env.local` (gitignored). NextAuth derives AUTH_SECRET in dev if
 * unset, but we always set NEXTAUTH_SECRET for consistency.
 *
 * Captcha verification is handled separately in
 * src/app/api/captcha/verify/route.ts and gated by CAPTCHA_ENABLED.
 *
 * On every successful sign-in we upsert the user against the
 * chakramcp-app backend (NEXT_PUBLIC_APP_API_URL, defaults to
 * http://localhost:8080) and stash the backend's user_id, is_admin
 * flag, and short-lived JWT inside the NextAuth JWT cookie. The
 * session callback surfaces those for client + server use.
 */

import NextAuth, { type DefaultSession } from "next-auth";
import Credentials from "next-auth/providers/credentials";
import GitHub from "next-auth/providers/github";
import Google from "next-auth/providers/google";

import { ApiClientError, loginWithPassword, upsertUser } from "@/lib/api";

declare module "next-auth" {
  interface Session {
    backendToken?: string;
    user: {
      id?: string;
      is_admin?: boolean;
    } & DefaultSession["user"];
  }
}

/** Loose extra fields stored on the NextAuth JWT during callbacks. */
type JwtExtras = {
  backendToken?: string;
  backendUserId?: string;
  isAdmin?: boolean;
  provider?: string;
};

export const { handlers, signIn, signOut, auth } = NextAuth({
  // NextAuth v5 reads AUTH_SECRET by default; we also accept NEXTAUTH_SECRET
  // for parity with v4 .env files and our .env.example.
  secret: process.env.AUTH_SECRET ?? process.env.NEXTAUTH_SECRET,
  providers: [
    GitHub({
      clientId: process.env.GITHUB_CLIENT_ID,
      clientSecret: process.env.GITHUB_CLIENT_SECRET,
      // Same email across GitHub + Google should be the same account.
      // The backend's users table is keyed on lowercased email and
      // collapses duplicates on upsert.
      allowDangerousEmailAccountLinking: true,
    }),
    Google({
      clientId: process.env.GOOGLE_CLIENT_ID,
      clientSecret: process.env.GOOGLE_CLIENT_SECRET,
      allowDangerousEmailAccountLinking: true,
    }),
    Credentials({
      id: "password",
      name: "Email and password",
      credentials: {
        email: { label: "Email", type: "email" },
        password: { label: "Password", type: "password" },
      },
      async authorize(creds) {
        const email = String(creds?.email ?? "").trim();
        const password = String(creds?.password ?? "");
        if (!email || !password) return null;

        try {
          const result = await loginWithPassword({ email, password });
          // Return shape that NextAuth's User expects, plus extras the
          // jwt callback below will pluck onto the JWT.
          return {
            id: result.user.id,
            email: result.user.email,
            name: result.user.display_name,
            image: result.user.avatar_url,
            backendToken: result.token,
            backendUserId: result.user.id,
            isAdmin: result.user.is_admin,
          } as unknown as Parameters<typeof Credentials>[0]["authorize"] extends (
            ...args: infer _a
          ) => Promise<infer _r>
            ? _r
            : never;
        } catch (err) {
          if (err instanceof ApiClientError) {
            console.error("[auth] password login failed", err.status, err.message);
          } else {
            console.error("[auth] password login error", err);
          }
          return null;
        }
      },
    }),
  ],
  pages: {
    signIn: "/login",
  },
  session: { strategy: "jwt" },
  trustHost: true,
  callbacks: {
    async signIn({ user, account, profile }) {
      if (!account || !user.email) return false;

      // Credentials provider: backend auth already happened in authorize().
      // The user object already has backendToken / isAdmin attached.
      if (account.provider === "password") {
        return true;
      }

      try {
        const result = await upsertUser({
          email: user.email,
          name: user.name ?? user.email,
          avatar_url: user.image ?? null,
          provider: account.provider,
          provider_user_id: account.providerAccountId,
          raw_profile: profile ?? null,
        });
        // Stash backend response on `user` so the jwt callback can
        // pick it up on first sign-in.
        (user as unknown as Record<string, unknown>).backendToken = result.token;
        (user as unknown as Record<string, unknown>).backendUserId = result.user.id;
        (user as unknown as Record<string, unknown>).isAdmin = result.user.is_admin;
      } catch (err) {
        if (err instanceof ApiClientError) {
          // Backend down or misconfigured - log and bounce to /login with an error.
          console.error("[auth] backend upsert failed", err.status, err.message);
        } else {
          console.error("[auth] upsert error", err);
        }
        return false;
      }
      return true;
    },

    async jwt({ token, user, account }) {
      const t = token as unknown as JwtExtras & Record<string, unknown>;
      // First sign-in: copy backend session bits onto the JWT.
      if (user) {
        const u = user as unknown as JwtExtras;
        if (u.backendToken) t.backendToken = u.backendToken;
        if (u.backendUserId) t.backendUserId = u.backendUserId;
        if (typeof u.isAdmin === "boolean") t.isAdmin = u.isAdmin;
      }
      if (account) {
        t.provider = account.provider;
      }
      return token;
    },

    async session({ session, token }) {
      const t = token as unknown as JwtExtras;
      session.backendToken = t.backendToken;
      session.user.id = t.backendUserId ?? session.user.id;
      session.user.is_admin = t.isAdmin ?? false;
      return session;
    },
  },
});
