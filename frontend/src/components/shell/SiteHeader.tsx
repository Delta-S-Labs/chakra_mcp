"use client";

import Link from "next/link";
import { usePathname } from "next/navigation";
import Brandmark from "./Brandmark";

const tabs = [
  { label: "Portfolio", href: "/" },
  { label: "Concept", href: "/concept" },
  { label: "Brand", href: "/brand" },
];

export default function SiteHeader() {
  const pathname = usePathname();

  // Portfolio is a public landing. Concept/brand pages are shared explicitly —
  // we don't advertise them from the portfolio. Hide the nav on "/" entirely.
  const showNav = pathname !== "/";
  const isActive = (href: string) =>
    href === "/" ? pathname === "/" : pathname.startsWith(href);

  return (
    <header className="site-header">
      <Link href="/" aria-label="ChakraMCP home" style={{ textDecoration: "none" }}>
        <Brandmark />
      </Link>
      {showNav && (
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
      )}
    </header>
  );
}
