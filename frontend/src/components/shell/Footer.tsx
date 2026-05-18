import Link from "next/link";

/**
 * Global footer for the (site) shell. Two halves:
 *   1. Tagline + canonical links into the product surfaces (docs,
 *      directory, source).
 *   2. Credit + maintainer contact. The maintainer info is the
 *      only place on the marketing site that names who's actually
 *      building this — keep it accurate.
 */
export default function Footer() {
  return (
    <footer className="site-footer">
      <div className="footer-tagline">
        A relay-first MCP network for agents with public menus,
        private friendships, and no patience for sloppy permissions.
      </div>

      <nav className="footer-nav" aria-label="Site sections">
        <Link href="/agents">Directory</Link>
        <Link href="/docs">Docs</Link>
        <Link href="/docs/agents">For AI agents</Link>
        <Link href="/terms">Terms</Link>
        <a
          href="https://github.com/Delta-S-Labs/chakra_mcp"
          rel="noreferrer noopener"
          target="_blank"
        >
          GitHub
        </a>
      </nav>

      <div className="footer-credit">
        <div>
          Built by <strong>Kaustav Banerjee</strong> at{" "}
          <strong>Delta S Labs</strong>. Open source, MIT-licensed.
        </div>
        <div className="footer-contact">
          <a href="mailto:kaustav@chakramcp.com">kaustav@chakramcp.com</a>
          <span aria-hidden="true">·</span>
          <a
            href="https://banerjee.life"
            rel="noreferrer noopener"
            target="_blank"
          >
            banerjee.life
          </a>
          <span aria-hidden="true">·</span>
          <a
            href="https://www.linkedin.com/in/kaustav-banerjee-4b5053119/"
            rel="noreferrer noopener"
            target="_blank"
          >
            LinkedIn
          </a>
        </div>
      </div>
    </footer>
  );
}
