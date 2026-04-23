import type { Metadata } from "next";
import Link from "next/link";
import RelayDiagram from "@/components/shell/RelayDiagram";

export const metadata: Metadata = {
  title: "Concept — ChakraMCP",
  description:
    "The protocol shape: object model, proposal lifecycle, consent modes, relay behavior, and what we build next.",
};

const objects: [string, string][] = [
  ["Account", "Represents an individual or organization that owns agents, members, and trust ceilings."],
  ["Member", "A human user inside an account who may act through a local agent using that agent\u2019s granted permissions."],
  ["Agent", "A registered MCP endpoint with metadata, maintainers, capability catalog, and policy settings."],
  ["Capability", "A public record for a tool or workflow, including visibility, execution mode, constraints, and consent rules."],
  ["Friendship", "A mutual relationship between accounts that enables further directional access grants."],
  ["Grant", "A directional permission from a target agent to a requester source agent, with optional constraints."],
  ["ConsentRecord", "Evidence that a sensitive capability was approved for a specific run, time window, or persistent unlock."],
  ["ActorContext", "The runtime caller envelope: requester account, source agent, and optional acting member."],
];

const proposal = [
  "A requester selects one target agent, picks capabilities, and submits an access proposal.",
  "The network evaluates auto-grant rules. Out-of-band requests route to maintainers or admins.",
  "Reviewers can accept, reduce, reject, or counteroffer. Broader grants need explicit re-acceptance.",
  "First accepted proposal creates account-level friendship. Ongoing access stays directional.",
  "Later proposals expand, shrink, or revoke access without rebuilding the relationship.",
];

const consent = [
  "Per invocation: every single run waits for approval.",
  "Time-boxed: approval opens a temporary window for repeated use.",
  "Persistent until revoked: a durable unlock that can still be pulled later.",
];

const vision = [
  {
    marker: "Next",
    title: "Managed agent runtime.",
    body: "A creator-friendly platform where you describe an agent, configure tools and knowledge, set guardrails, and publish — we run the sandbox, sessions, and scaling. Same idea as Anthropic\u2019s Managed Agents, shaped for consumer creators.",
  },
  {
    marker: "Then",
    title: "Token economy.",
    body: "Earn by watching ads or renting idle device compute. Spend on AI usage. Creators accumulate tokens through agent usage and cash out through standard payment rails. No crypto on day one.",
  },
  {
    marker: "After",
    title: "Creator marketplace.",
    body: "Discoverable catalog of agents, creator analytics, in-agent purchases, creator-sourced advertisers. 10% platform cut — lower than Apple, lower than Google, high-volume bet.",
  },
  {
    marker: "Later",
    title: "Distributed compute network.",
    body: "Users contribute idle CPU/GPU to run small local models. Earn tokens passively. Cheaper inference shifts the cost curve of the whole platform.",
  },
];

export default function ConceptPage() {
  return (
    <>
      <section className="hero-block hero-block--concept">
        <div className="hero-copy reveal">
          <div className="eyebrow">Concept page</div>
          <h1>A relay-first protocol for agents who want to talk to strangers.</h1>
          <p className="lead">
            Public catalog. Negotiated access. Consent-aware runtime. The network checks identity,
            scope, and audit rules before any remote tool or workflow runs.
          </p>
          <div className="hero-actions">
            <Link className="pill-link pill-link--primary" href="/brand">
              Brand + assets
            </Link>
            <Link className="pill-link" href="/">
              Back to portfolio
            </Link>
          </div>
        </div>
        <aside className="hero-board reveal">
          <div className="note-badge">Paperwork, not magic</div>
          <RelayDiagram />
        </aside>
      </section>

      <section className="concept-at-a-glance">
        <div className="eyebrow">At a glance</div>
        <div className="glance-grid">
          <div className="glance-card">
            <h3>Public catalog</h3>
            <p>Every agent can advertise exactly what it does, including what becomes available only after friendship.</p>
          </div>
          <div className="glance-card">
            <h3>Negotiated access</h3>
            <p>Friendship creates the relationship. Directional grants decide what one side may actually use from the other.</p>
          </div>
          <div className="glance-card">
            <h3>Relay enforcement</h3>
            <p>The network checks identity, scope, consent, and audit rules before any remote tool or workflow runs.</p>
          </div>
        </div>
      </section>

      <section className="concept-stage">
        <div className="chapter-marker">01</div>
        <div className="concept-stage__body">
          <div className="section-head">
            <div className="eyebrow">Objects</div>
            <h2>Eight things the system knows about.</h2>
          </div>
          <dl className="object-grid object-grid--staggered">
            {objects.map(([term, body]) => (
              <div className="object-item" key={term}>
                <dt>{term}</dt>
                <dd>{body}</dd>
              </div>
            ))}
          </dl>
        </div>
      </section>

      <section className="concept-stage">
        <div className="chapter-marker">02</div>
        <div className="concept-stage__body">
          <div className="section-head">
            <div className="eyebrow">Flow</div>
            <h2>How a proposal becomes a grant.</h2>
          </div>
          <ol className="flow-list flow-list--concept">
            {proposal.map((s, i) => (
              <li className="flow-step" key={i}>
                <p>{s}</p>
              </li>
            ))}
          </ol>
        </div>
      </section>

      <section className="concept-stage">
        <div className="chapter-marker">03</div>
        <div className="concept-stage__body">
          <div className="section-head">
            <div className="eyebrow">Consent</div>
            <h2>Three modes, all revocable.</h2>
          </div>
          <div className="consent-band">
            {consent.map((c, i) => (
              <div className="consent-mode" key={i}>
                <p>{c}</p>
              </div>
            ))}
          </div>
        </div>
      </section>

      <section className="concept-stage">
        <div className="chapter-marker">04</div>
        <div className="concept-stage__body">
          <div className="section-head">
            <div className="eyebrow">Vision</div>
            <h2>What we build next, on top of the relay.</h2>
            <p>
              The relay network is the spine. Everything below is the muscle. Each layer depends on
              the one beneath it. Each one makes the ones above it more valuable.
            </p>
          </div>
          <div className="highlight-grid">
            {vision.map((v) => (
              <article className="highlight-tile" key={v.title}>
                <div className="eyebrow">{v.marker}</div>
                <h3>{v.title}</h3>
                <p>{v.body}</p>
              </article>
            ))}
          </div>
        </div>
      </section>

      <section className="closing-panel">
        <div className="eyebrow">MVP boundary</div>
        <h2>v1 is the relay. Runtime, economy, marketplace, and compute are follow-ons.</h2>
        <p>
          The backend is Rust on AWS (ECS Fargate + RDS Postgres). Auth via JWT, webhook delivery
          signed with HMAC-SHA256, and an event system with at-least-once delivery and idempotency.
        </p>
      </section>
    </>
  );
}
