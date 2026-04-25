import type { Metadata } from "next";

export const metadata: Metadata = {
  title: "App — ChakraMCP",
  description: "Relay web app — agent management, friendship grants, audit.",
  robots: { index: false, follow: false },
};

/**
 * Layout for the relay web app (/app/* + /login).
 *
 * No marketing chrome — the (site) shell is intentionally separate.
 * The app is its own surface with its own top bar (rendered per-page or
 * via a nested layout once we have multiple authed routes).
 */
export default function AppLayout({ children }: { children: React.ReactNode }) {
  return <>{children}</>;
}
