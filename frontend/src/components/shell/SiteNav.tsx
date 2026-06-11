"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";

/**
 * Public-site navigation strip, sibling of the brandmark in `<SiteHeader />`.
 *
 * Renders one of three shapes depending on auth state + current path:
 *
 *   • brandmark-only path (/concept, /brand, /cofounder) → renders nothing
 *   • logged out → single "Sign in" link to /login
 *   • logged in  → Docs (/docs) + App (/app) tabs, in that order
 *
 * Active-state highlighting follows the existing `.nav-link.active`
 * style so we don't need any new CSS to land this. Per the SiteHeader
 * file comment, the `isLoggedIn` bit is computed server-side and
 * passed in — we never call useSession from here.
 */
export default function SiteNav({
  isLoggedIn,
  brandmarkOnlyPaths,
}: {
  isLoggedIn: boolean;
  brandmarkOnlyPaths: string[];
}) {
  const pathname = usePathname();

  // A brandmark-only page suppresses every link in the strip — that
  // page wants the brandmark to stand alone (see SiteHeader's
  // BRANDMARK_ONLY set for the rationale).
  if (brandmarkOnlyPaths.includes(pathname)) {
    return null;
  }

  const isActive = (href: string) =>
    href === "/" ? pathname === "/" : pathname.startsWith(href);

  // Everyone gets the content tabs; the last slot depends on auth
  // state — "App" for signed-in users, "Sign in" for visitors.
  const tabs = [
    { label: "Use cases", href: "/use-cases" },
    { label: "FAQ", href: "/faq" },
    { label: "Docs", href: "/docs" },
    isLoggedIn ? { label: "App", href: "/app" } : { label: "Sign in", href: "/login" },
  ];

  return (
    <nav className="site-nav" aria-label="Primary">
      {tabs.map((t) => (
        <Link
          key={t.href}
          href={t.href}
          className={"nav-link" + (isActive(t.href) ? " active" : "")}
        >
          {t.label}
        </Link>
      ))}
    </nav>
  );
}
