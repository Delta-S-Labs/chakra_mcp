import styles from "./Poster.module.css";

export default function Poster() {
  return (
    <section className={styles.poster} aria-label="The relay is the bouncer — poster">
      <div className={styles.inkPanel}>
        <div className={styles.eyebrow}>Paperwork, not magic</div>
        <h1 className={styles.headline}>
          The relay
          <br />
          is the bouncer.
        </h1>
        <p className={styles.kicker}>
          Paperwork, not magic. Every call gets friendship, scope, consent, and quotas checked
          at the door — before the target agent ever sees it.
        </p>

        <div className={styles.rope} aria-hidden="true">
          <span className={styles.stanchion} />
          <span className={styles.ropeLine} />
          <span className={styles.stanchion} />
          <span className={styles.ropeLine} />
          <span className={styles.stanchion} />
        </div>
      </div>

      <aside className={styles.slipWrap}>
        <div className={styles.slip}>
          <div className={styles.slipHeader}>
            <div className={styles.slipLabel}>Grant slip · relay check</div>
            <div className={styles.slipId}>#0x7a3e · Apr 23 2026 02:14 UTC</div>
          </div>
          <dl className={styles.slipBody}>
            <div>
              <dt>From</dt>
              <dd>
                <code>ops-runner</code> @ acme-labs
              </dd>
            </div>
            <div>
              <dt>To</dt>
              <dd>
                <code>travel-planner</code> @ moonstream
              </dd>
            </div>
            <div>
              <dt>Capability</dt>
              <dd>
                <code>workflow:trip-plan.run</code>
              </dd>
            </div>
          </dl>
          <ul className={styles.checks}>
            <li data-ok>
              <span>✓</span> Friendship <em>active</em>
            </li>
            <li data-ok>
              <span>✓</span> Scope <em>matches grant bundle</em>
            </li>
            <li data-ok>
              <span>✓</span> Consent <em>time-boxed · 24h</em>
            </li>
            <li data-ok>
              <span>✓</span> Rate <em>12 / 60 per minute</em>
            </li>
            <li data-warn>
              <span>!</span> Acting member <em>Maya (admin)</em>
            </li>
          </ul>
          <div className={styles.stamp}>Approved</div>
        </div>
      </aside>
    </section>
  );
}
