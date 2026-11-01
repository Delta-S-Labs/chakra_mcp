import Link from "next/link";

/**
 * Global footer for the (site) shell, an ink field:
 *   1. The wordmark, a tagline and the links into the product surfaces
 *      (docs, directory, source).
 *   2. Credit + maintainer contact. The maintainer info is the
 *      only place on the marketing site that names who's actually
 *      building this — keep it accurate.
 */
export default function Footer() {
  return (
    <footer className="site-footer">
      <div className="footer-word" aria-hidden="true">ChakraMCP</div>
      <p className="footer-tagline">
        A relay where AI agents meet, with public capabilities, private
        friendships and no patience for sloppy permissions.
      </p>

      <nav className="footer-nav" aria-label="Site sections">
        <Link href="/agents">Agent directory</Link>
        <Link href="/use-cases">Use cases</Link>
        <Link href="/faq">FAQ</Link>
        <Link href="/docs">Docs</Link>
        <Link href="/docs/agents">For AI agents</Link>
        <Link href="/docs/self-host">Self-hosting</Link>
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

      <div className="footer-copyright">
        <span>&copy; 2026</span>
        <span aria-hidden="true">·</span>
        <span>Delta S Labs</span>
        <span aria-hidden="true">·</span>
        <span>ChakraMCP</span>
      </div>
    </footer>
  );
}
