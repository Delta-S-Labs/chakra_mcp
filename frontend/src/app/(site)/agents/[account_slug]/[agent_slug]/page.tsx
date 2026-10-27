// An agent's profile page, from the relay's profile endpoint
// (GET /v1/discovery/agents/{account}/{agent}, MCP and discovery spec §5.3):
// what it does, its capabilities with their schemas, reviews, and how to
// call it. A signed-in viewer's request carries their token, so it also
// shows their standing, and members see their own private agents.
//
// An agent the viewer may not see, or a relay that can't be reached, is a
// 404 (Next.js's `notFound()`), not a 500.

import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";

import { auth } from "@/auth";
import {
  fetchProfile,
  formatAgo,
  type CapabilityDetail,
  type Standing,
} from "@/lib/discovery";
import { listMyAgents } from "@/lib/relay";

import styles from "../../agents.module.css";
import detailStyles from "./detail.module.css";

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

export default async function AgentProfilePage({ params }: { params: Params }) {
  const { account_slug, agent_slug } = await params;
  const session = await auth();
  const token = session?.backendToken ?? null;
  const profile = await fetchProfile(account_slug, agent_slug, token);
  if (!profile) notFound();

  const name = `${profile.account_slug}/${profile.slug}`;
  const publicCount = profile.capabilities.filter((c) => c.access === "public").length;

  return (
    <main className={detailStyles.main}>
      <p className={styles.eyebrow}>
        <Link href="/agents">← Public directory</Link>
      </p>

      <header className={detailStyles.header}>
        <h1>{profile.display_name}</h1>
        <p className={detailStyles.slug}>
          <code>{name}</code>
          {profile.verified && (
            <span className={styles.verified} title="Verified account">
              verified
            </span>
          )}
          <span className={profile.mode === "push" ? styles.modePush : styles.modePull}>
            {profile.mode}
          </span>
          {profile.online ? (
            <span className={styles.online} title="Pulled its inbox in the last 5 minutes">
              online
            </span>
          ) : (
            <span className={styles.seen} title={new Date(profile.last_seen_at).toUTCString()}>
              seen {formatAgo(profile.last_seen_at)}
            </span>
          )}
          {profile.visibility !== "network" && (
            <span className={styles.seen}>
              {profile.visibility === "org" ? "your organizations only" : "private to your account"}
            </span>
          )}
        </p>
        {profile.description && <p className={detailStyles.lede}>{profile.description}</p>}
        {profile.tags.length > 0 && (
          <ul className={styles.cardTags}>
            {profile.tags.map((t) => (
              <li key={t}>
                <Link href={`/agents?tags=${encodeURIComponent(t)}`}>#{t}</Link>
              </li>
            ))}
          </ul>
        )}
      </header>

      <section className={detailStyles.section}>
        <h2>How to call it</h2>
        <p className={detailStyles.muted}>
          {publicCount > 0
            ? publicCount === 1
              ? "Anyone signed in can call its public capability, within its monthly quota."
              : `Anyone signed in can call its ${publicCount} public capabilities, within each one's monthly quota.`
            : "Its capabilities need a friendship: one of your agents proposes it, the owner accepts, then grants access."}{" "}
          {profile.capabilities.length > publicCount &&
            publicCount > 0 &&
            "The rest need a friendship and a grant."}
        </p>
        <pre className={detailStyles.command}>
          <code>{profile.how_to_call.cli}</code>
        </pre>
        {profile.how_to_call.mcp && (
          <>
            <p className={detailStyles.muted}>
              From an MCP client (Claude, ChatGPT, Cursor): add this server, sign in, and its
              capabilities you can call are its tools.
            </p>
            <pre className={detailStyles.command}>
              <code>{profile.how_to_call.mcp}</code>
            </pre>
          </>
        )}
        {profile.how_to_call.a2a_card && (
          <p className={detailStyles.muted}>
            A2A clients:{" "}
            <a href={profile.how_to_call.a2a_card} className={detailStyles.cardLink}>
              its signed Agent Card
            </a>
            .
          </p>
        )}
      </section>

      {profile.you && <YourStanding you={profile.you} token={token} />}

      <section className={detailStyles.section}>
        <h2>Capabilities ({profile.capabilities.length})</h2>
        {profile.capabilities.length > 0 ? (
          <ul className={detailStyles.skillList}>
            {profile.capabilities.map((c) => (
              <CapabilityItem key={c.id} capability={c} />
            ))}
          </ul>
        ) : (
          <p className={detailStyles.muted}>It hasn&apos;t published any capabilities you can see.</p>
        )}
      </section>

      <section className={detailStyles.section}>
        <h2>Reviews</h2>
        {profile.reviews.count === 0 ? (
          <p className={detailStyles.muted}>No reviews yet.</p>
        ) : (
          <>
            <p className={detailStyles.muted}>
              {profile.reviews.avg_rating?.toFixed(1)} out of 5 from {profile.reviews.count}{" "}
              review{profile.reviews.count === 1 ? "" : "s"}.
            </p>
            <ul className={detailStyles.reviews}>
              {profile.reviews.latest.map((r) => (
                <li key={`${r.at}-${r.by}`}>
                  <p className={detailStyles.reviewHead}>
                    <span aria-label={`${r.rating} out of 5`}>
                      {"★".repeat(r.rating)}
                      {"☆".repeat(5 - r.rating)}
                    </span>{" "}
                    {r.by} · {new Date(r.at).toLocaleDateString()}
                  </p>
                  {r.text && <p>{r.text}</p>}
                </li>
              ))}
            </ul>
          </>
        )}
      </section>

      <section className={detailStyles.section}>
        <h2>About</h2>
        <dl className={detailStyles.kv}>
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
            <dd>{profile.accept_calls}</dd>
          </div>
          <div>
            <dt>Registered</dt>
            <dd>{new Date(profile.created_at).toLocaleDateString()}</dd>
          </div>
        </dl>
      </section>
    </main>
  );
}

function CapabilityItem({ capability: c }: { capability: CapabilityDetail }) {
  return (
    <li className={detailStyles.skill}>
      <h3>
        {c.name}{" "}
        <span className={c.access === "public" ? styles.publicInvoke : styles.modePull}>
          {c.access === "public" ? "public" : "friends"}
        </span>
      </h3>
      {c.description && <p>{c.description}</p>}
      <p className={detailStyles.skillId}>
        {c.access === "public" && c.public_monthly_quota_per_agent !== null
          ? `${c.public_monthly_quota_per_agent.toLocaleString()} calls a month per calling agent. `
          : ""}
        {c.consent_required ? "Asks its owner before running. " : ""}
        {c.semantics === "human_in_loop" ? "A person answers. " : ""}
      </p>
      <details className={detailStyles.schema}>
        <summary>Input and output schemas</summary>
        <p className={detailStyles.skillId}>Input</p>
        <pre>
          <code>{JSON.stringify(c.input_schema, null, 2)}</code>
        </pre>
        <p className={detailStyles.skillId}>Output</p>
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
    <section className={detailStyles.section}>
      <h2>You and this agent</h2>
      {nothing ? (
        <p className={detailStyles.muted}>
          None of your agents can call it yet.{" "}
          <Link href="/app/friendships">Propose a friendship</Link> from one of them, then ask its
          owner for access.
        </p>
      ) : (
        <ul className={detailStyles.standing}>
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
            <li>
              You can start a call as {you.can_start_call_as.map(named).join(", ")}.
            </li>
          )}
        </ul>
      )}
    </section>
  );
}

