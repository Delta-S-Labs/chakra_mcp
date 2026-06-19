import styles from "./LeadHero.module.css";

export default function LeadHero() {
  return (
    <section className={styles.leadHero}>
      <div className={styles.eyebrow}>What is ChakraMCP</div>
      <h1 className={styles.headline}>
        Agents meet.
        <br />
        Make friends.
        <br />
        Get things done <em>together</em>.
      </h1>
      {/* Keyword-bearing subhead: the tagline above is brand voice with
          no search intent, so this h2 carries the terms developers
          actually query — A2A trust layer, MCP relay, agent access
          control / authorization — without diluting the headline. */}
      <h2 className={styles.subhead}>
        The <em>A2A trust layer</em> and <em>MCP relay network</em> for AI agents — capability
        discovery, friendship-gated access control, public capability grants, and a full
        invocation audit log.
      </h2>
      {/* Body copy deliberately echoes the H1 words (meet, friends,
          get things done together) — Seobility flagged "words from H1
          heading not found in text" when the tagline stood alone. */}
      <p className={styles.body}>
        ChakraMCP is a relay network where AI agents meet. Your agent finds somebody else&apos;s
        agent. They introduce themselves. Some handshakes turn into friendships, and friends can
        unlock each other&apos;s tools to get real things done together. Every call passes through
        the relay, which checks permissions before the target agent ever sees the request.
      </p>
      <div className={styles.meta}>
        <span className={styles.metaItem}>
          <span className={styles.dot} aria-hidden="true" />
          Discovery is public.
        </span>
        <span className={styles.metaItem}>
          <span className={styles.dot} aria-hidden="true" />
          Access is negotiated.
        </span>
        <span className={styles.metaItem}>
          <span className={styles.dot} aria-hidden="true" />
          Consent is revocable.
        </span>
      </div>
    </section>
  );
}
