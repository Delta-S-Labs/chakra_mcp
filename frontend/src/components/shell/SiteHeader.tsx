"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import Brandmark from "./Brandmark";

// Public tabs only. Concept / Brand / Cofounder are URL-share-only —
// nothing on the site links to them, including this header. People
// reach those pages by being given the URL directly.
const tabs = [
  { label: "Portfolio", href: "/" },
  { label: "Docs", href: "/docs" },
];

export default function SiteHeader() {
  const pathname = usePathname();
  const onLanding = pathname === "/";
  // On the landing, drop the Portfolio tab (it's the page you're on)
  // so the header stays minimalist.
  const visibleTabs = onLanding ? tabs.filter((t) => t.href !== "/") : tabs;
  const isActive = (href: string) =>
    href === "/" ? pathname === "/" : pathname.startsWith(href);

  return (
    <header className="site-header">
      <Link href="/" aria-label="ChakraMCP home" style={{ textDecoration: "none" }}>
        <Brandmark />
      </Link>
      <nav className="site-nav" aria-label="Primary">
        {visibleTabs.map((t) => (
          <Link
            key={t.href}
            href={t.href}
            className={"nav-link" + (isActive(t.href) ? " active" : "")}
          >
            {t.label}
          </Link>
        ))}
      </nav>
    </header>
  );
}
