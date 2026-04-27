import type { Metadata } from "next";
import Link from "next/link";
import RelayDiagram from "@/components/shell/RelayDiagram";
import styles from "./concept.module.css";

export const metadata: Metadata = {
  title: "Concept \u2014 ChakraMCP",
  description:
    "The protocol shape: object model, proposal lifecycle, consent modes, relay behavior, and what we build next on top of the relay.",
  robots: { index: false, follow: false },
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

const stackLayers = [
  {
    id: 5,
    label: "Distributed compute network",
    note: "Users rent out idle device compute. The platform gets cheaper inference. Everyone earns.",
    status: "later",
  },
  {
    id: 4,
    label: "Creator marketplace + revenue share",
    note: "Discoverable catalog, creator analytics, in-agent purchases, creator-sourced advertisers.",
    status: "after",
  },
  {
    id: 3,
    label: "Token economy",
    note: "Earn tokens by watching ads or renting compute. Spend tokens on AI usage. Creators cash out.",
    status: "then",
  },
  {
    id: 2,
    label: "Managed agent runtime",
    note: "Describe an agent, configure knowledge and tools, publish. We run the sandbox and scaling.",
    status: "next",
  },
  {
    id: 1,
    label: "ChakraMCP relay network",
    note: "Trust, discovery, friendship, grants, consent, audit. The spine everything else stands on.",
    status: "here",
  },
];

const visionLayers = [
  {
    eyebrow: "Layer 2 \u00b7 Next",
    title: "Managed agent runtime.",
    body: "A creator-friendly platform where you describe an agent and we handle everything else \u2014 the runtime, sandboxed execution, session continuity, error recovery, scaling. Creators never touch a server.",
    bullets: [
      "Three tiers of tools: platform-provided, creator-authenticated, user-authenticated.",
      "Three LLM modes: bring-your-own keys, user-picks-the-model, platform-wrapped.",
      "Off-platform agents: your infrastructure, your LLM, still a first-class network citizen.",
      "Knowledge bases stored in object storage with vector indexing for retrieval.",
    ],
  },
  {
    eyebrow: "Layer 3 \u00b7 Then",
    title: "Token economy.",
    body: "A universal platform currency that flows between every participant. Earn by watching ads or renting idle device compute. Spend on AI usage. Creators accumulate tokens from agent usage and cash out through standard payment rails (Stripe, bank transfer).",
    bullets: [
      "Phase 1 \u2014 Internal credit. Ads for tokens. Creator payouts via revenue share.",
      "Phase 2 \u2014 Fiat on-ramp. Buy tokens with money. No withdrawal yet.",
      "Phase 3 \u2014 Real liquidity. Tokens become withdrawable, potentially tradeable.",
      "No crypto on day one. Build the economy first. Tokenize when volume justifies it.",
    ],
  },
  {
    eyebrow: "Layer 4 \u00b7 After",
    title: "Creator marketplace.",
    body: "A discovery and distribution platform where creators publish agents, users find and use them, and the token economy handles the revenue flow. Think YouTube but for AI agents.",
    bullets: [
      "Ad revenue share during free-tier agent sessions.",
      "In-agent purchases with a 10% platform cut \u2014 lower than Apple, lower than Google.",
      "Creator-sourced advertisers: creators bring their own brand relationships onto the platform.",
      "Anti-fraud protections on ratings and reviews.",
    ],
  },
  {
    eyebrow: "Layer 5 \u00b7 Later",
    title: "Distributed compute network.",
    body: "A decentralized compute layer where users contribute idle CPU, memory, and GPU to run small local LLMs for the platform. Users earn tokens passively. The platform gets cheaper inference. The cost curve shifts.",
    bullets: [
      "Lightweight cross-platform runtime distributed via Tauri or similar.",
      "Model distribution via torrent-style P2P to avoid bandwidth costs.",
      "Result verification through probabilistic cross-checking.",
      "Only suitable for latency-tolerant workloads \u2014 not for real-time chat.",
    ],
  },
];

const flywheels = [
  {
    name: "The attention flywheel",
    loop: [
      "More users",
      "more ad revenue",
      "more creator payouts",
      "more creators",
      "more agents",
      "more reasons to use",
      "more users",
    ],
  },
  {
    name: "The compute flywheel",
    loop: [
      "More users",
      "more device compute",
      "cheaper inference",
      "better economics",
      "lower barrier for creators",
      "more agents",
      "more users",
    ],
  },
  {
    name: "The network flywheel",
    loop: [
      "More users",
      "more external agents connect",
      "bigger catalog",
      "more reasons to stay",
      "more users",
    ],
  },
];

const timeline = [
  { quarter: "Q2 2026", milestone: "ChakraMCP relay network v1 live. Rust on AWS (ECS Fargate + RDS Postgres)." },
  { quarter: "Q3 2026", milestone: "Managed agent runtime MVP. First 10 creator-built agents." },
  { quarter: "Q4 2026", milestone: "Token economy live. Ad integration. Free tier opens." },
  { quarter: "Q1 2027", milestone: "Creator marketplace. Public launch. Seed fundraise." },
  { quarter: "Q2\u2013Q3 2027", milestone: "Premium subscriptions. In-agent purchases. Scale creators." },
  { quarter: "Q4 2027", milestone: "Creator-sourced advertisers. Off-platform agent integration." },
  { quarter: "2028", milestone: "Distributed compute pilot. Token liquidity exploration." },
];

const shipsFirst = [
  "Agent registration (create, update, retire).",
  "Discovery (search by name, tags, description, capability).",
  "Access requests \u2014 direct, no counteroffers yet.",
  "Grant acceptance \u2014 directional, scoped.",
  "Sync relay execution \u2014 tool calls forwarded through the relay.",
  "Audit log \u2014 every invocation recorded.",
];

const shipsLater = [
  "Full friendship model with counteroffer and negotiation.",
  "Consent modes (per-invocation, time-boxed, persistent).",
  "Webhook delivery (polling only in v1).",
  "Async job lifecycle and capability runs.",
  "Acting-member context and admin check.",
  "Secret rotation with overlap window.",
  "Rate limiting.",
  "MCP-native transport (HTTP REST in v1).",
];

const revenuePhases = [
  {
    phase: "Phase 1",
    title: "Ad revenue",
    items: [
      "Banner ads in the free tier (persistent, low CPM, high volume).",
      "15\u201330s video and audio interstitials between sessions.",
      "Native in-feed ads in the marketplace, matched to platform design.",
      "Sponsored creator placements.",
    ],
  },
  {
    phase: "Phase 2",
    title: "Premium subscriptions",
    items: [
      "Ad-free experience with a monthly token allowance.",
      "Priority access to popular creators.",
      "Higher usage limits.",
    ],
  },
  {
    phase: "Phase 3",
    title: "Platform economics",
    items: [
      "In-agent purchases \u2014 10% platform cut, 90% creator.",
      "Creator-sourced advertiser collaborations.",
      "Token purchases via fiat on-ramp.",
      "Enterprise API access for high-volume integrators.",
    ],
  },
  {
    phase: "Phase 4",
    title: "Compute economics",
    items: [
      "Spread between token cost to users and compute cost from distributed devices.",
      "Premium inference tiers (faster models, guaranteed latency).",
    ],
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
            <div className="eyebrow">The stack</div>
            <h2>The relay is the spine. Everything else is muscle.</h2>
            <p>
              Each layer depends on the one beneath it. Each layer makes the ones above it more
              valuable. The relay is infrastructure \u2014 it is not the product users see, not the
              business model, and not the thing that generates revenue. But nothing above it can
              exist without it.
            </p>
          </div>
          <div className={styles.stack}>
            {stackLayers.map((layer) => (
              <div
                key={layer.id}
                className={`${styles.stackLayer} ${styles[`stackLayer--${layer.status}`]}`}
                data-status={layer.status}
              >
                <div className={styles.stackLayerNum}>0{layer.id}</div>
                <div className={styles.stackLayerBody}>
                  <div className={styles.stackLayerLabel}>{layer.label}</div>
                  <div className={styles.stackLayerNote}>{layer.note}</div>
                </div>
                {layer.status === "here" && (
                  <div className={styles.stackLayerPin}>We are here</div>
                )}
              </div>
            ))}
          </div>
        </div>
      </section>

      <section className="concept-stage">
        <div className="chapter-marker">05</div>
        <div className="concept-stage__body">
          <div className="section-head">
            <div className="eyebrow">Vision</div>
            <h2>What we build next on top of the relay.</h2>
          </div>
          <div className={styles.visionGrid}>
            {visionLayers.map((v) => (
              <article key={v.title} className={styles.visionCard}>
                <div className="eyebrow">{v.eyebrow}</div>
                <h3>{v.title}</h3>
                <p>{v.body}</p>
                <ul>
                  {v.bullets.map((b) => (
                    <li key={b}>{b}</li>
                  ))}
                </ul>
              </article>
            ))}
          </div>
        </div>
      </section>

      <section className="concept-stage">
        <div className="chapter-marker">06</div>
        <div className="concept-stage__body">
          <div className="section-head">
            <div className="eyebrow">The flywheel</div>
            <h2>Three interlocking loops. Each one accelerates the others.</h2>
          </div>
          <div className={styles.flywheels}>
            {flywheels.map((f) => (
              <article key={f.name} className={styles.flywheelCard}>
                <div className="eyebrow">{f.name}</div>
                <div className={styles.flywheelLoop}>
                  {f.loop.map((step, i) => (
                    <span key={i} className={styles.flywheelStep}>
                      {step}
                    </span>
                  ))}
                </div>
              </article>
            ))}
          </div>
        </div>
      </section>

      <section className="concept-stage">
        <div className="chapter-marker">07</div>
        <div className="concept-stage__body">
          <div className="section-head">
            <div className="eyebrow">Revenue model</div>
            <h2>Phased streams, not a single plan.</h2>
          </div>
          <div className={styles.revenueGrid}>
            {revenuePhases.map((p) => (
              <article key={p.phase} className={styles.revenueCard}>
                <div className={styles.revenuePhase}>{p.phase}</div>
                <h3>{p.title}</h3>
                <ul>
                  {p.items.map((item) => (
                    <li key={item}>{item}</li>
                  ))}
                </ul>
              </article>
            ))}
          </div>
        </div>
      </section>

      <section className="concept-stage">
        <div className="chapter-marker">08</div>
        <div className="concept-stage__body">
          <div className="section-head">
            <div className="eyebrow">Timeline</div>
            <h2>The next 24 months, quarter by quarter.</h2>
          </div>
          <ol className={styles.timeline}>
            {timeline.map((t) => (
              <li key={t.quarter} className={styles.timelineItem}>
                <div className={styles.timelineQuarter}>{t.quarter}</div>
                <div className={styles.timelineBody}>{t.milestone}</div>
              </li>
            ))}
          </ol>
        </div>
      </section>

      <section className="concept-stage">
        <div className="chapter-marker">09</div>
        <div className="concept-stage__body">
          <div className="section-head">
            <div className="eyebrow">Who this is for, right now</div>
            <h2>Two kinds of early users.</h2>
          </div>
          <div className={styles.portraits}>
            <article className={styles.portrait}>
              <p>
                A senior engineer at a mid-stage startup. They&apos;ve been asked to build a
                multi-agent workflow. Their agents need to call another team&apos;s agents.
                They&apos;ve spent three weeks on auth middleware, a capability registry, and
                audit logging &mdash; none of it the actual product. They want to delete the
                trust layer and go back to building features.
              </p>
            </article>
            <article className={styles.portrait}>
              <p>
                Or: someone running a personal agent of their own &mdash; a Hermes instance, an
                OpenClaw, something self-hosted. They want their agent to meet other agents, trade
                capabilities, handle requests from their friends&apos; agents &mdash; without
                writing the trust plumbing from scratch, and without handing their agent to a
                walled garden.
              </p>
            </article>
            <div className={styles.portraitFoot}>
              That&apos;s who this is for, right now.
            </div>
          </div>
        </div>
      </section>

      <section className="concept-stage">
        <div className="chapter-marker">10</div>
        <div className="concept-stage__body">
          <div className="section-head">
            <div className="eyebrow">What ships first</div>
            <h2>Six capabilities. Everything else is v2.</h2>
            <p>
              The v1 relay is the thing we ship to prove that developers will use a managed
              relay instead of rebuilding the trust layer themselves. Everything below gets
              added once v1 has real traffic.
            </p>
          </div>
          <div className={styles.shipsGrid}>
            <article className={styles.shipsCol + " " + styles.shipsFirst}>
              <div className={styles.shipsHead}>v1 ships</div>
              <ul>
                {shipsFirst.map((s) => (
                  <li key={s}>
                    <span className={styles.shipsTick}>✓</span> {s}
                  </li>
                ))}
              </ul>
            </article>
            <article className={styles.shipsCol + " " + styles.shipsLater}>
              <div className={styles.shipsHead}>v1 skips</div>
              <ul>
                {shipsLater.map((s) => (
                  <li key={s}>
                    <span className={styles.shipsCross}>–</span> {s}
                  </li>
                ))}
              </ul>
            </article>
          </div>
        </div>
      </section>

      <section className={styles.bet}>
        <div className={styles.betInner}>
          <div className="eyebrow">The bet</div>
          <h2 className={styles.betHeadline}>
            Attention is a valid currency for AI access \u2014 the same way it funds music, video,
            and news.
          </h2>
          <p>
            Spotify proved you can fund music with ads. YouTube proved you can fund video with ads
            and a creator economy. Most of the internet proved you can fund news the same way. We
            are betting the same mechanics work for AI access, and that the first platform to nail
            free AI at scale will have a structural advantage that subscriptions-only competitors
            cannot replicate.
          </p>
          <p>
            ChakraMCP is the foundation \u2014 the trust and communication layer that makes
            agent-to-agent collaboration possible without a human babysitting every handshake.
            Everything we build on top of it is in service of one idea:
          </p>
          <p className={styles.betPunch}>AI shouldn\u2019t cost $20/month. We\u2019re fixing that.</p>
        </div>
      </section>

    </>
  );
}
