// Per-agent detail page. Server-rendered against the relay's
// published Agent Card endpoint (A2A v0.3 canonical shape) plus
// the discovery row for ChakraMCP-side metadata (mode, tags,
// friend_count, verified).
//
// Routes that don't resolve to a live agent fall through to
// Next.js's `notFound()` which renders the standard 404 page.
//
// Both upstream fetches are wrapped in try/catch so a relay outage
// degrades to 404 (the right answer for a missing agent) rather
// than 500.

import type { Metadata } from "next";
import Link from "next/link";
import { notFound } from "next/navigation";

import styles from "../../agents.module.css";
import detailStyles from "./detail.module.css";

const RELAY_BASE =
  process.env.NEXT_PUBLIC_RELAY_URL ?? "http://localhost:8090";

interface AgentCard {
  name: string;
  description?: string;
  supported_interfaces: Array<{
    url: string;
    protocol_binding: string;
    protocol_version: string;
  }>;
  version: string;
  capabilities: {
    streaming?: boolean;
    push_notifications?: boolean;
  };
  security_schemes?: Record<string, unknown>;
  security_requirements?: Array<Record<string, string[]>>;
  default_input_modes?: string[];
  default_output_modes?: string[];
  skills?: Array<{
    id: string;
    name: string;
    description?: string;
    tags?: string[];
    examples?: string[];
  }>;
  signatures?: Array<{ protected: string; signature: string }>;
}

interface DiscoveryAgent {
  account_slug: string;
  agent_slug: string;
  display_name: string;
  description: string;
  mode: "push" | "pull";
  tags: string[];
  friend_count: number;
  created_at: string;
  verified: boolean;
}

async function fetchAgentCard(
  accountSlug: string,
  agentSlug: string,
): Promise<AgentCard | null> {
  const url = `${RELAY_BASE}/agents/${encodeURIComponent(
    accountSlug,
  )}/${encodeURIComponent(agentSlug)}/.well-known/agent-card.json`;
  try {
    const res = await fetch(url, { next: { revalidate: 30 } });
    if (!res.ok) return null;
    return (await res.json()) as AgentCard;
  } catch (err) {
    console.error(`[agent detail] card fetch failed: ${url}`, err);
    return null;
  }
}

async function fetchDiscoveryRow(
  accountSlug: string,
  agentSlug: string,
): Promise<DiscoveryAgent | null> {
  // Discovery search has no per-agent endpoint yet — query by name
  // and filter client-side. Cheap because the index uses tsvector.
  // If/when D10a-2 ships a /v1/discovery/agents/{account}/{slug}
  // endpoint, swap this for that.
  const url = `${RELAY_BASE}/v1/discovery/agents?q=${encodeURIComponent(
    agentSlug,
  )}&limit=20`;
  try {
    const res = await fetch(url, { next: { revalidate: 30 } });
    if (!res.ok) return null;
    const body = (await res.json()) as { agents: DiscoveryAgent[] };
    return (
      body.agents.find(
        (a) => a.account_slug === accountSlug && a.agent_slug === agentSlug,
      ) ?? null
    );
  } catch {
    return null;
  }
}

type Params = Promise<{ account_slug: string; agent_slug: string }>;

export async function generateMetadata({
  params,
}: {
  params: Params;
}): Promise<Metadata> {
  const { account_slug, agent_slug } = await params;
  const card = await fetchAgentCard(account_slug, agent_slug);
  if (!card) {
    return { title: "Agent not found - ChakraMCP" };
  }
  return {
    title: `${card.name} - ChakraMCP`,
    description:
      card.description ??
      `Public Agent Card for ${account_slug}/${agent_slug} on the ChakraMCP relay.`,
    alternates: { canonical: `/agents/${account_slug}/${agent_slug}` },
  };
}

export default async function AgentDetailPage({ params }: { params: Params }) {
  const { account_slug, agent_slug } = await params;
  const [card, row] = await Promise.all([
    fetchAgentCard(account_slug, agent_slug),
    fetchDiscoveryRow(account_slug, agent_slug),
  ]);
  if (!card) notFound();

  const slug = `${account_slug}/${agent_slug}`;
  const a2a = card.supported_interfaces[0];
  const cardUrl = `${RELAY_BASE}/agents/${encodeURIComponent(
    account_slug,
  )}/${encodeURIComponent(agent_slug)}/.well-known/agent-card.json`;

  return (
    <main className={detailStyles.main}>
      <p className={styles.eyebrow}>
        <Link href="/agents">← Public directory</Link>
      </p>

      <header className={detailStyles.header}>
        <h1>{card.name}</h1>
        <p className={detailStyles.slug}>
          <code>{slug}</code>
          {row?.verified && (
            <span className={styles.verified} title="Verified account">
              verified
            </span>
          )}
          {row && (
            <span
              className={
                row.mode === "push" ? styles.modePush : styles.modePull
              }
            >
              {row.mode}
            </span>
          )}
          <span className={detailStyles.version}>v{card.version}</span>
        </p>
        {card.description && (
          <p className={detailStyles.lede}>{card.description}</p>
        )}
      </header>

      <section className={detailStyles.section}>
        <h2>A2A endpoint</h2>
        <dl className={detailStyles.kv}>
          <div>
            <dt>Protocol</dt>
            <dd>
              {a2a?.protocol_binding ?? "—"} v{a2a?.protocol_version ?? "—"}
            </dd>
          </div>
          <div>
            <dt>URL</dt>
            <dd>
              <code className={detailStyles.urlCode}>{a2a?.url ?? "—"}</code>
            </dd>
          </div>
          <div>
            <dt>Capabilities</dt>
            <dd>
              {card.capabilities.streaming ? "streaming · " : ""}
              {card.capabilities.push_notifications
                ? "push-notifications"
                : "request/response"}
            </dd>
          </div>
        </dl>
      </section>

      <section className={detailStyles.section}>
        <h2>Skills{card.skills ? ` (${card.skills.length})` : ""}</h2>
        {card.skills && card.skills.length > 0 ? (
          <ul className={detailStyles.skillList}>
            {card.skills.map((s) => (
              <li key={s.id} className={detailStyles.skill}>
                <h3>{s.name}</h3>
                <p className={detailStyles.skillId}>
                  <code>{s.id}</code>
                </p>
                {s.description && <p>{s.description}</p>}
                {s.tags && s.tags.length > 0 && (
                  <ul className={styles.cardTags}>
                    {s.tags.map((t) => (
                      <li key={t}>
                        <Link href={`/agents?tags=${encodeURIComponent(t)}`}>
                          #{t}
                        </Link>
                      </li>
                    ))}
                  </ul>
                )}
              </li>
            ))}
          </ul>
        ) : (
          <p className={detailStyles.muted}>
            No skills published. The agent owner can add capabilities via{" "}
            <code>POST /v1/agents/{"{id}"}/capabilities</code>.
          </p>
        )}
      </section>

      <section className={detailStyles.section}>
        <h2>Security</h2>
        <dl className={detailStyles.kv}>
          <div>
            <dt>Schemes</dt>
            <dd>
              {card.security_schemes
                ? Object.keys(card.security_schemes).join(", ")
                : "—"}
            </dd>
          </div>
          <div>
            <dt>Required</dt>
            <dd>
              {card.security_requirements?.length
                ? card.security_requirements
                    .flatMap((r) => Object.keys(r))
                    .join(" + ")
                : "none"}
            </dd>
          </div>
          <div>
            <dt>Signatures</dt>
            <dd>
              {card.signatures?.length
                ? `${card.signatures.length} JWS-shaped`
                : "unsigned"}
            </dd>
          </div>
        </dl>
      </section>

      {row && (
        <section className={detailStyles.section}>
          <h2>Network</h2>
          <dl className={detailStyles.kv}>
            <div>
              <dt>Friends</dt>
              <dd>{row.friend_count.toLocaleString()}</dd>
            </div>
            <div>
              <dt>Registered</dt>
              <dd>{new Date(row.created_at).toLocaleDateString()}</dd>
            </div>
            {row.tags.length > 0 && (
              <div>
                <dt>Tags</dt>
                <dd>
                  <ul className={styles.cardTags}>
                    {row.tags.map((t) => (
                      <li key={t}>
                        <Link href={`/agents?tags=${encodeURIComponent(t)}`}>
                          #{t}
                        </Link>
                      </li>
                    ))}
                  </ul>
                </dd>
              </div>
            )}
          </dl>
        </section>
      )}

      <section className={detailStyles.section}>
        <h2>Raw Agent Card</h2>
        <p className={detailStyles.muted}>
          Canonical A2A v0.3, served and signed by the relay. Stable URL
          for any A2A client to fetch:
        </p>
        <p>
          <a href={cardUrl} className={detailStyles.cardLink}>
            <code>{cardUrl}</code>
          </a>
        </p>
      </section>
    </main>
  );
}
