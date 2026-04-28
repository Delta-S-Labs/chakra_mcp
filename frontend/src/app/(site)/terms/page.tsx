import type { Metadata } from "next";
import Link from "next/link";
import styles from "./terms.module.css";

export const metadata: Metadata = {
  title: "Terms - ChakraMCP",
  description:
    "Plain-English terms for signing in and using the ChakraMCP relay network.",
};

const lastUpdated = "2026-04-25";
const version = "v0.1";

export default function TermsPage() {
  return (
    <div className={styles.page}>
      <header className={styles.header}>
        <div className="eyebrow">Terms</div>
        <h1 className={styles.title}>When you sign in, here&apos;s what you agree to.</h1>
        <p className={styles.lead}>
          Plain English. We&apos;ll add a more formal version when we need one. This is the spirit
          of the contract - and what we hold ourselves to.
        </p>
      </header>

      <section className={styles.stage}>
        <h2>What we collect</h2>
        <p>
          Your OAuth profile from your provider: name, email, avatar URL. That&apos;s it for
          sign-in. We don&apos;t read your DMs, your repos, your contacts, or anything else your
          provider would offer to share.
        </p>
      </section>

      <section className={styles.stage}>
        <h2>What we don&apos;t collect</h2>
        <ul>
          <li>
            <strong>The contents of agent-to-agent messages.</strong> The relay routes them; it
            doesn&apos;t open them.
          </li>
          <li>
            <strong>Your private agent fields.</strong> Your agent decides what gets shared per
            grant. We never see the rest.
          </li>
          <li>
            <strong>Third-party trackers on the auth surface.</strong> No analytics pixels on
            <code> /login</code> or <code>/app</code>.
          </li>
        </ul>
      </section>

      <section className={styles.stage}>
        <h2>What we do with what we do collect</h2>
        <p>
          Identify you so your agents are yours. Show your name and avatar in the dashboard. Send
          a transactional email if there&apos;s an outage or your account is deleted. That&apos;s
          the list.
        </p>
      </section>

      <section className={styles.stage}>
        <h2>What you can do, anytime</h2>
        <ul>
          <li>Delete your account (button on the dashboard once that surface lands).</li>
          <li>Revoke any grant your agent has issued.</li>
          <li>
            Email <a href="mailto:kaustav@banerjee.life">kaustav@banerjee.life</a> with anything
            else.
          </li>
        </ul>
      </section>

      <section className={styles.stage}>
        <h2>What we can do</h2>
        <ul>
          <li>
            Update these terms when the product changes. We bump the version and the date at the
            bottom of this page when we do.
          </li>
          <li>
            Suspend an account that&apos;s clearly abusing the relay - spam, scraping, hammering
            APIs from inside agent calls. We&apos;d rather not, and we&apos;ll notify before we
            do.
          </li>
        </ul>
      </section>

      <section className={styles.stage}>
        <h2>Open source</h2>
        <p>
          The relay code is{" "}
          <a href="https://github.com/Delta-S-Labs/chakra_mcp">open source under MIT</a> - you can
          self-host the relay inside a company, inside a private network, on your laptop. The terms
          on this page apply to the managed public network only. Self-hosted networks set their own.
        </p>
      </section>

      <footer className={styles.foot}>
        <p>
          {version} · last updated {lastUpdated} · the public history lives in the{" "}
          <a
            href="https://github.com/Delta-S-Labs/chakra_mcp/commits/main/frontend/src/app/(site)/terms/page.tsx"
            target="_blank"
            rel="noreferrer"
          >
            git log
          </a>
          .
        </p>
      </footer>
    </div>
  );
}
