import type { ReactNode } from "react";
import styles from "./Examples.module.css";

type ExampleProps = {
  caption?: string;
  children: ReactNode;
};

/**
 * One example inside the Examples section. Takes any self-contained visual
 * as children, renders it with an optional caption underneath.
 */
function Example({ caption, children }: ExampleProps) {
  return (
    <article className={styles.example}>
      <div className={styles.exampleMedia}>{children}</div>
      {caption && <p className={styles.caption}>{caption}</p>}
    </article>
  );
}

type ExamplesProps = {
  children: ReactNode;
  /** Eyebrow label above the headline. */
  eyebrow?: string;
  /**
   * Heading element for the section headline. The landing page used to
   * mount this under its own h1, so it defaulted to h2; the standalone
   * /use-cases page passes "h1" to make the headline the page title.
   */
  headingAs?: "h1" | "h2";
};

export default function Examples({ children, eyebrow = "Examples", headingAs = "h2" }: ExamplesProps) {
  const Heading = headingAs;
  return (
    <section className={styles.section} aria-labelledby="examples-heading">
      <header className={styles.sectionHead}>
        <div className={styles.sectionEyebrow}>{eyebrow}</div>
        <Heading id="examples-heading" className={styles.sectionHeadline}>
          What this looks like in practice.
        </Heading>
        <p className={styles.sectionLead}>
          A few stories from the network. Some are routine. Some are the kind of thing that used
          to need a human in the loop at 3am.
        </p>
      </header>
      <div className={styles.exampleList}>{children}</div>
    </section>
  );
}

Examples.Item = Example;
