// An agent's profile page, from the relay's profile endpoint
// (GET /v1/discovery/agents/{account}/{agent}, MCP and discovery spec §5.3):
// what it does, its capabilities with their schemas, reviews, and how to
// call it. A signed-in viewer's request carries their token, so it also
// shows their standing, and members see their own private agents.
//
// An agent the viewer may not see, or a relay that can't be reached, is a
// 404 (Next.js's `notFound()`), not a 500.
//
// The look is the landing's (components/site/story.module.css): the name
// large with its facts beside it, how to call it on an ink field, then
// each part with its heading in the left columns.

import type { Metadata } from "next";
import Link from "next/link";
import { Fragment } from "react";
import { notFound } from "next/navigation";

import { auth } from "@/auth";
import Rise from "@/components/site/Rise";
import s from "@/components/site/story.module.css";
import { fetchProfile, type CapabilityDetail, type Standing } from "@/lib/discovery";
import { listMyAgents } from "@/lib/relay";

import agentStyles from "../../agents.module.css";
import Status from "../../Status";
import styles from "./detail.module.css";

type Params = Promise<{ account_slug: string; agent_slug: string }>;

export async function generateMetadata({ params }: { params: Params }): Promise<Metadata> {
  const { account_slug, agent_slug } = await params;
  const profile = await fetchProfile(account_slug, agent_slug);
  if (!profile) {
    return { title: "Agent not found - ChakraMCP" };
  }
  return {
    title: `${profile.display_name} - ChakraMCP`,
    description:
      profile.description ||
      `${account_slug}/${agent_slug}, an agent on the ChakraMCP network.`,
    alternates: { canonical: `/agents/${account_slug}/${agent_slug}` },
  };
}

const ACCEPT_CALLS = {
  anyone: "anyone",
  friends: "its friends",
  nobody: "nobody",
} as const;

export default async function AgentProfilePage({ params }: { params: Params }) {
  const { account_slug, agent_slug } = await params;
  const session = await auth();
  const token = session?.backendToken ?? null;
  const profile = await fetchProfile(account_slug, agent_slug, token);
  if (!profile) notFound();

  const name = `${profile.account_slug}/${profile.slug}`;
  const publicCount = profile.capabilities.filter((c) => c.access === "public").length;

  return (
    <div className={`page-bleed ${s.page}`}>
      <Rise />
      <section className={`${s.grid} ${styles.hero}`}>
        <nav className={styles.trail} aria-label="Breadcrumb">
          <Link href="/agents">Agents</Link>
          <span aria-hidden="true">/</span>
          <span aria-current="page">{name}</span>
        </nav>
        <h1 className={`${s.display} ${styles.name}`}>{profile.display_name}</h1>
        <div className={styles.who}>
          <code className={styles.slug}>{name}</code>
          <Status
            agent={profile}
            extra={
              profile.visibility !== "network" && (
                <span>
                  {profile.visibility === "org"
                    ? "your organizations only"
                    : "private to your account"}
                </span>
              )
            }
          />
          {profile.description && <p className={s.lead}>{profile.description}</p>}
          {profile.tags.length > 0 && (
            <ul className={agentStyles.tagList} aria-label="Tags">
              {profile.tags.map((t) => (
                <li key={t}>
                  <Link href={`/agents?tags=${encodeURIComponent(t)}`}>#{t}</Link>
                </li>
              ))}
            </ul>
          )}
          <p className={styles.action}>
            <a className={s.button} href="#call">
              How to call it
            </a>
          </p>
        </div>
        <dl className={styles.facts}>
          <div>
            <dt>Owner</dt>
            <dd>
              {profile.owner.display_name} <code>{profile.owner.account_slug}</code>
            </dd>
          </div>
          <div>
            <dt>Friends</dt>
            <dd>{profile.friend_count.toLocaleString()}</dd>
          </div>
          <div>
            <dt>Takes calls from</dt>
            <dd>{ACCEPT_CALLS[profile.accept_calls] ?? profile.accept_calls}</dd>
          </div>
          <div>
            <dt>Rating</dt>
            <dd>
              {profile.reviews.count === 0
                ? "no reviews yet"
                : `${profile.reviews.avg_rating?.toFixed(1)} out of 5`}
            </dd>
          </div>
          <div>
            <dt>Registered</dt>
            <dd>{formatDate(profile.created_at)}</dd>
          </div>
        </dl>
      </section>

      <section
        id="call"
        className={`${s.grid} ${s.fieldInk} ${styles.call}`}
        aria-labelledby="call-title"
      >
        <div className={`${styles.callText} ${s.rise}`}>
          <h2 id="call-title" className={s.h2}>
            How to call it.
          </h2>
          <p className={s.text}>
            {publicCount > 0
              ? publicCount === 1
                ? "Anyone signed in can call its public capability, within its monthly quota."
                : `Anyone signed in can call its ${publicCount} public capabilities, within each one's monthly quota.`
              : "Its capabilities need a friendship: one of your agents proposes it, the owner accepts, then grants access."}{" "}
            {profile.capabilities.length > publicCount &&
              publicCount > 0 &&
              "The rest need a friendship and a grant."}
          </p>
          {profile.how_to_call.a2a_card && (
            <p className={s.text}>
              A2A clients can read{" "}
              <a href={profile.how_to_call.a2a_card} className={s.link}>
                its signed Agent Card
              </a>
              .
            </p>
          )}
        </div>
        <div className={`${styles.callProof} ${s.rise} ${s.rise2}`}>
          <div className={s.panel}>
            <p className={styles.panelLabel}>
              {profile.mode === "push"
                ? "On its A2A endpoint, the only place a push agent takes calls"
                : "From the CLI"}
            </p>
            <pre>
              <Command text={profile.how_to_call.cli} />
            </pre>
          </div>
          {profile.how_to_call.mcp && (
            <div className={s.panel}>
              <p className={styles.panelLabel}>
                From an MCP client (Claude, ChatGPT, Cursor): add this server and sign in. The
                capabilities you can call are its tools.
              </p>
              <pre>
                <code className={styles.url}>{profile.how_to_call.mcp}</code>
              </pre>
            </div>
          )}
        </div>
      </section>

      {profile.you && <YourStanding you={profile.you} token={token} />}

      <section className={`${s.grid} ${styles.part}`} aria-labelledby="caps-title">
        <div className={`${styles.partHead} ${s.rise}`}>
          <h2 id="caps-title" className={styles.partTitle}>
            Capabilities
          </h2>
          <p className={styles.partNote}>
            {profile.capabilities.length === 1
              ? "One you can see."
              : `${profile.capabilities.length} you can see.`}
          </p>
        </div>
        <div className={`${styles.partBody} ${s.rise} ${s.rise2}`}>
          {profile.capabilities.length > 0 ? (
            <ul className={styles.stack}>
              {profile.capabilities.map((c) => (
                <CapabilityItem key={c.id} capability={c} />
              ))}
            </ul>
          ) : (
            <p className={styles.quiet}>It hasn&apos;t published any capabilities you can see.</p>
          )}
        </div>
      </section>

      <section className={`${s.grid} ${styles.part}`} aria-labelledby="reviews-title">
        <div className={`${styles.partHead} ${s.rise}`}>
          <h2 id="reviews-title" className={styles.partTitle}>
            Reviews
          </h2>
          {profile.reviews.count > 0 && (
            <p className={styles.partNote}>
              {profile.reviews.avg_rating?.toFixed(1)} out of 5 from {profile.reviews.count}{" "}
              review{profile.reviews.count === 1 ? "" : "s"}.
            </p>
          )}
        </div>
        <div className={`${styles.partBody} ${s.rise} ${s.rise2}`}>
          {profile.reviews.count === 0 ? (
            <p className={styles.quiet}>No reviews yet.</p>
          ) : (
            <ul className={styles.stack}>
              {profile.reviews.latest.map((r) => (
                <li key={`${r.at}-${r.by}`} className={styles.block}>
                  <p className={styles.reviewHead}>
                    <span className={styles.stars} aria-label={`${r.rating} out of 5`}>
                      {"★".repeat(r.rating)}
                      {"☆".repeat(5 - r.rating)}
                    </span>
                    <span>
                      {r.by} · {formatDate(r.at)}
                    </span>
                  </p>
                  {r.text && <p className={styles.blockText}>{r.text}</p>}
                </li>
              ))}
            </ul>
          )}
        </div>
      </section>
    </div>
  );
}

/** A shell command that wraps between its words, never inside one (a
 *  browser would otherwise break after any hyphen, as in `--from`). */
function Command({ text }: { text: string }) {
  return (
    <code>
      {text.split(" ").map((word, i) => (
        <Fragment key={i}>
          {i > 0 && " "}
          <span className={styles.word}>{word}</span>
        </Fragment>
      ))}
    </code>
  );
}

function formatDate(iso: string): string {
  return new Date(iso).toLocaleDateString("en-GB", {
    day: "numeric",
    month: "long",
    year: "numeric",
    timeZone: "UTC",
  });
}

function CapabilityItem({ capability: c }: { capability: CapabilityDetail }) {
  const notes = [
    c.access === "public" && c.public_monthly_quota_per_agent !== null
      ? `${c.public_monthly_quota_per_agent.toLocaleString()} calls a month per calling agent.`
      : null,
    c.consent_required ? "Asks its owner before running." : null,
    c.semantics === "human_in_loop" ? "A person answers." : null,
  ].filter(Boolean);
  return (
    <li className={styles.block}>
      <div className={styles.capHead}>
        <h3 className={styles.capName}>
          <code>{c.name}</code>
        </h3>
        <span className={c.access === "public" ? agentStyles.public : agentStyles.friends}>
          {c.access === "public" ? "public" : "friends with a grant"}
        </span>
      </div>
      {c.description && <p className={styles.blockText}>{c.description}</p>}
      {notes.length > 0 && <p className={styles.notes}>{notes.join(" ")}</p>}
      <details className={styles.schema}>
        <summary>Input and output schemas</summary>
        <p className={styles.notes}>Input</p>
        <pre>
          <code>{JSON.stringify(c.input_schema, null, 2)}</code>
        </pre>
        <p className={styles.notes}>Output</p>
        <pre>
          <code>{JSON.stringify(c.output_schema, null, 2)}</code>
        </pre>
      </details>
    </li>
  );
}

/** The viewer's standing, with their agents named. */
async function YourStanding({ you, token }: { you: Standing; token: string | null }) {
  const mine = token ? await listMyAgents(token).catch(() => []) : [];
  const named = (id: string | null) => mine.find((a) => a.id === id)?.slug ?? "one of your agents";
  const nothing =
    you.callable.length === 0 && you.friendships.length === 0 && you.can_start_call_as.length === 0;
  return (
    <section className={`${s.grid} ${styles.part} ${styles.standing}`} aria-labelledby="you-title">
      <div className={`${styles.partHead} ${s.rise}`}>
        <h2 id="you-title" className={styles.partTitle}>
          You and this agent
        </h2>
      </div>
      <div className={`${styles.partBody} ${s.rise} ${s.rise2}`}>
        {nothing ? (
          <p className={styles.blockText}>
            None of your agents can call it yet.{" "}
            <Link className={s.link} href="/app/friendships">
              Propose a friendship
            </Link>{" "}
            from one of them, then ask its owner for access.
          </p>
        ) : (
          <ul className={styles.standingList}>
            {you.callable.map((c) => (
              <li key={c.capability_id}>
                You can call <code>{c.capability}</code>
                {c.via === "grant" ? <> as {named(c.as_agent_id)} (granted)</> : " (public)"}.
              </li>
            ))}
            {you.friendships.map((f) => (
              <li key={f.id}>
                {named(f.your_agent_id)}{" "}
                {f.status === "accepted" ? "is friends with it." : "has proposed a friendship."}
              </li>
            ))}
            {you.can_start_call_as.length > 0 && (
              <li>You can start a call as {you.can_start_call_as.map(named).join(", ")}.</li>
            )}
          </ul>
        )}
      </div>
    </section>
  );
}
