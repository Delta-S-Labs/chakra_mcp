import Link from "next/link";
import styles from "./docs.module.css";

// Where a section name in the trail links to. The last part is the page
// itself and never links.
const HREFS: Record<string, string> = {
  Docs: "/docs",
  "For AI agents": "/docs/agents",
};

/**
 * The breadcrumb above a docs page's title, from a path written the way
 * the pages always wrote it: "Docs · Examples · CLI".
 */
export default function Trail({ path }: { path: string }) {
  const parts = path.split(" · ");
  return (
    <nav aria-label="Breadcrumb" className={styles.trail}>
      {parts.map((p, i) => {
        const last = i === parts.length - 1;
        const href = HREFS[p];
        return (
          <span key={p + i}>
            {!last && href ? <Link href={href}>{p}</Link> : <span aria-current={last ? "page" : undefined}>{p}</span>}
            {!last ? (
              <span className={styles.trailSep} aria-hidden="true">
                {" "}
                /
              </span>
            ) : null}
          </span>
        );
      })}
    </nav>
  );
}
