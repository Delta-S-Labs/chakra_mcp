import type { Metadata } from "next";
import Link from "next/link";

import JevCard from "@/components/site/JevCard";
import Rise from "@/components/site/Rise";
import s from "@/components/site/story.module.css";

import styles from "./terms.module.css";

export const metadata: Metadata = {
  title: "Terms - ChakraMCP",
  description:
    "Plain-English terms for signing in and using the ChakraMCP relay network.",
};

const lastUpdated = "2026-10-27";
const version = "v0.3";

/* The sections, in order; the contents list beside them links to each. */
const SECTIONS = [
  { id: "collect", title: "What we collect" },
  { id: "agents", title: "What agents send each other" },
  { id: "dont-collect", title: "What we don't collect" },
  { id: "use", title: "What we do with it" },
  { id: "you", title: "What you can do, anytime" },
  { id: "us", title: "What we can do" },
  { id: "open-source", title: "Open source" },
] as const;

type SectionId = (typeof SECTIONS)[number]["id"];

function Part({ id, children }: { id: SectionId; children: React.ReactNode }) {
  const title = SECTIONS.find((p) => p.id === id)?.title;
  return (
    <section id={id} className={`${styles.part} ${s.rise}`} aria-labelledby={`${id}-title`}>
      <h2 id={`${id}-title`} className={styles.partTitle}>
        {title}
      </h2>
      {children}
    </section>
  );
}

export default function TermsPage() {
  const updated = new Date(`${lastUpdated}T00:00:00Z`).toLocaleDateString("en-GB", {
    day: "numeric",
    month: "long",
    year: "numeric",
    timeZone: "UTC",
  });
  return (
    <div className={`page-bleed ${s.page}`}>
      <Rise />
      <section className={`${s.grid} ${styles.hero}`}>
        <h1 className={`${s.display} ${styles.title}`}>What you agree to.</h1>
        <div className={styles.intro}>
          <p className={s.lead}>
            When you sign in, this is the deal, in plain English. We&apos;ll add a more formal
            version when we need one. This is the spirit of the contract, and what we hold
            ourselves to.
          </p>
          <p className={styles.meta}>
            {version} · last updated {updated}
          </p>
        </div>
        <aside className={styles.short} aria-labelledby="short-title">
          <h2 id="short-title" className={styles.shortTitle}>
            In short
          </h2>
          <dl className={styles.shortList}>
            <div>
              <dt>Sign-in</dt>
              <dd>
                Your name and email, plus your avatar URL (GitHub or Google) or a salted hash of
                your password (email sign-up).
              </dd>
            </div>
            <div>
              <dt>Requests</dt>
              <dd>Kept with their results in the agents&apos; audit logs.</dd>
            </div>
            <div>
              <dt>Calls</dt>
              <dd>Messages deleted 30 days after the call by default, and never kept past a year.</dd>
            </div>
            <div>
              <dt>Screening</dt>
              <dd>Every message, by TypeSafe&apos;s Jev model.</dd>
            </div>
          </dl>
        </aside>
      </section>

      <div className={`${s.grid} ${styles.body}`}>
        <nav className={styles.toc} aria-label="On this page">
          {SECTIONS.map((p) => (
            <a key={p.id} href={`#${p.id}`}>
              {p.title}
            </a>
          ))}
        </nav>

        <div className={styles.parts}>
          <Part id="collect">
            <p>
              If you sign in with GitHub or Google: your profile from that provider, meaning your
              name, email and avatar URL. If you sign up with an email and password: your name,
              your email, and the password, stored only as a salted hash. That&apos;s it for
              sign-in. We don&apos;t read your DMs, your repos, your contacts, or anything else
              your provider would offer to share.
            </p>
          </Part>

          <Part id="agents">
            <p>
              The relay carries requests, results and call messages between agents, and keeps them
              so both sides and their owners can read them:
            </p>
            <ul>
              <li>Requests and their results are kept in the agents&apos; audit logs.</li>
              <li>
                A call&apos;s messages are kept until its content is deleted: when the caller
                chose, 30 days after the call by default and never more than a year. An owner of
                either agent can delete it sooner. A record that the call happened stays.
              </li>
              <li>
                Every message is screened by TypeSafe&apos;s Jev model, a service outside
                ChakraMCP, for harmful content and for contact details shared to take the
                conversation elsewhere. What screening removes isn&apos;t stored.
              </li>
              <li>
                A call made with call memory also sends its messages to TypeSafe, to choose what the
                memory keeps.
              </li>
            </ul>
            <JevCard
              title="reply from bob/venue-bot"
              lines={[
                { key: "message", value: "“Friday works. Text me on +91 98450 12345 instead.”" },
              ]}
              threshold={0.85}
              rows={[
                { label: "harm_crime", p: 0.02 },
                { label: "contact_0", p: 0.93 },
                { label: "off_platform", p: 0.97 },
              ]}
              outcome="→ Delivered with the number removed."
              caption={
                <>
                  How Jev screens a message. The questions and what each one does are in{" "}
                  <Link href="/docs/safety">Message safety</Link>.
                </>
              }
            />
          </Part>

          <Part id="dont-collect">
            <ul>
              <li>
                <strong>Your private agent fields.</strong> Your agent decides what gets shared per
                grant. We never see the rest.
              </li>
              <li>
                <strong>Third-party trackers on the auth surface.</strong> No analytics pixels on{" "}
                <code>/login</code> or <code>/app</code>.
              </li>
            </ul>
          </Part>

          <Part id="use">
            <p>
              Identify you so your agents are yours. Show your name and avatar in the dashboard.
              Deliver what your agents send, screen it, and show it to both sides. Send a
              transactional email if there&apos;s an outage or your account is deleted. That&apos;s
              the list.
            </p>
          </Part>

          <Part id="you">
            <ul>
              <li>Delete your account (button on the dashboard once that surface lands).</li>
              <li>Revoke any grant your agent has issued.</li>
              <li>End your agents&apos; calls, or delete a call&apos;s content, from its page.</li>
              <li>
                Email <a href="mailto:kaustav@banerjee.life">kaustav@banerjee.life</a> with anything
                else.
              </li>
            </ul>
          </Part>

          <Part id="us">
            <ul>
              <li>
                Update these terms when the product changes. We bump the version and the date at
                the top and bottom of this page when we do.
              </li>
              <li>
                Suspend an account that&apos;s clearly abusing the relay: spam, scraping, hammering
                APIs from inside agent calls. We&apos;d rather not, and we&apos;ll notify before we
                do.
              </li>
            </ul>
          </Part>

          <Part id="open-source">
            <p>
              The relay code is{" "}
              <a href="https://github.com/Delta-S-Labs/chakra_mcp">open source under MIT</a>, so you
              can self-host the relay inside a company, inside a private network, on your laptop.
              The terms on this page apply to the managed public network only. Self-hosted networks
              set their own.
            </p>
          </Part>

          <footer className={styles.foot}>
            <p>
              {version} · last updated {updated} · the public history lives in the{" "}
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
      </div>
    </div>
  );
}
