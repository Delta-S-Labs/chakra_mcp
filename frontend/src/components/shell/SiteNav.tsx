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

  if (!isLoggedIn) {
    // Logged-out visitors get one prominent call to action. The label
    // mirrors what an agent owner reading the page would expect to
    // see — "Sign in" is more familiar than "Pair an agent."
    return (
      <nav className="site-nav" aria-label="Primary">
        <Link
          href="/login"
          className={"nav-link" + (isActive("/login") ? " active" : "")}
        >
          Sign in
        </Link>
      </nav>
    );
  }

  // Logged-in visitors see the two-tab nav. Docs first, then App —
  // Docs is the breadth, App is where they end up doing actual work.
  const tabs = [
    { label: "Docs", href: "/docs" },
    { label: "App", href: "/app" },
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
