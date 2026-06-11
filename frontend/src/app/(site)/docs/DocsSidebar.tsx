"use client";

import { useMemo, useState } from "react";
import Link from "next/link";
import { usePathname } from "next/navigation";

import { DOCS_TABS, type DocsTab } from "./nav";
import styles from "./docs.module.css";

/**
 * Left-hand docs navigation (machine0/Mintlify-style):
 *
 *   - two audience tabs (For humans / For AI) — which one is active is
 *     derived from the URL, so deep links land on the right tree
 *   - a search box that filters the active tab's links by title/keywords
 *   - grouped link sections with active-page highlighting
 *
 * Mobile (<960px): the sidebar collapses behind a "Docs menu" disclosure
 * button; picking a link closes it again.
 */
export default function DocsSidebar() {
  const pathname = usePathname();
  const [query, setQuery] = useState("");
  const [open, setOpen] = useState(false);

  // /docs/agents and below = the AI tab; everything else = humans.
  const activeTabId: DocsTab["id"] = pathname.startsWith("/docs/agents") ? "ai" : "humans";
  const activeTab = DOCS_TABS.find((t) => t.id === activeTabId)!;

  const q = query.trim().toLowerCase();
  const groups = useMemo(() => {
    if (!q) return activeTab.groups;
    return activeTab.groups
      .map((g) => ({
        ...g,
        links: g.links.filter((l) =>
          `${l.title} ${l.keywords ?? ""}`.toLowerCase().includes(q),
        ),
      }))
      .filter((g) => g.links.length > 0);
  }, [activeTab, q]);

  const isActive = (href: string) => pathname === href;

  return (
    <>
      <button
        type="button"
        className={styles.menuToggle}
        aria-expanded={open}
        aria-controls="docs-sidebar"
        onClick={() => setOpen((v) => !v)}
      >
        <span aria-hidden="true">{open ? "✕" : "☰"}</span> Docs menu
      </button>

      <aside
        id="docs-sidebar"
        className={`${styles.sidebar} ${open ? styles.sidebarOpen : ""}`}
        aria-label="Docs navigation"
      >
        <div className={styles.tabRow} role="tablist" aria-label="Docs audience">
          {DOCS_TABS.map((tab) => (
            <Link
              key={tab.id}
              href={tab.rootHref}
              role="tab"
              aria-selected={tab.id === activeTabId}
              className={`${styles.tabBtn} ${tab.id === activeTabId ? styles.tabBtnActive : ""}`}
              onClick={() => {
                setQuery("");
                setOpen(false);
              }}
            >
              {tab.label}
            </Link>
          ))}
        </div>

        <input
          type="search"
          className={styles.search}
          placeholder="Search docs…"
          aria-label="Search docs"
          value={query}
          onChange={(e) => setQuery(e.target.value)}
        />

        {groups.length === 0 && (
          <p className={styles.searchEmpty}>
            Nothing here matches “{query}”.
          </p>
        )}

        {groups.map((g) => (
          <nav key={g.label} className={styles.group} aria-label={g.label}>
            <p className={styles.groupLabel}>{g.label}</p>
            <ul className={styles.navList}>
              {g.links.map((l) => (
                <li key={l.href}>
                  <Link
                    href={l.href}
                    aria-current={isActive(l.href) ? "page" : undefined}
                    className={`${styles.navLink} ${isActive(l.href) ? styles.navLinkActive : ""}`}
                    onClick={() => setOpen(false)}
                  >
                    {l.title}
                  </Link>
                </li>
              ))}
            </ul>
          </nav>
        ))}
      </aside>
    </>
  );
}
