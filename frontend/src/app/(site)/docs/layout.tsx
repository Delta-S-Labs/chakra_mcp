import DocsSidebar from "./DocsSidebar";
import styles from "./docs.module.css";

/**
 * Shared shell for every /docs/* page: a sticky left sidebar (audience
 * tabs + search + grouped nav) beside the prose column. Pages keep
 * rendering their own `<main className={styles.shell}>` — this layout
 * only owns the two-column frame and the mobile menu behaviour.
 */
export default function DocsLayout({ children }: { children: React.ReactNode }) {
  return (
    <div className={styles.docsLayout}>
      <DocsSidebar />
      <div className={styles.docsContent}>{children}</div>
    </div>
  );
}
