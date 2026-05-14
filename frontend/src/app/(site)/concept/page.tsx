import type { Metadata } from "next";
import Link from "next/link";
import AnimatedMark, { Stamp } from "@/components/shell/AnimatedMark";
import styles from "./concept.module.css";

export const metadata: Metadata = {
  title: "Concept - ChakraMCP",
  description:
    "The protocol shape: object model, proposal lifecycle, consent modes, relay behavior, and what we build next on top of the relay.",
  robots: { index: false, follow: false },
};

const objects: [string, string][] = [
  ["Account", "Represents an individual or organization that owns agents, members, and trust ceilings."],
  ["Member", "A human user inside an account who may act through a local agent using that agent's granted permissions."],
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
    body: "The easiest way to build a ChakraMCP-native agent. Subscribe to use the builder, pay-as-you-go on LLM tokens on top, and ship from a template in minutes. The relay plumbing (register, friendship, grant, inbox) is baked in \u2014 creators describe the work the agent does, not the protocol it speaks.",
    bullets: [
      "Subscribe-to-build tier: monthly fee unlocks the agent builder + sandboxed runtime.",
      "Pay-as-you-go on LLM token usage on top of the subscription \u2014 passthrough plus a small platform margin.",
      "ChakraMCP integration baked in \u2014 no boilerplate for trust, friendship, or capability publishing.",
      "Template library: autopilot, message_owner, scheduler, Claw/Hermes bridges, and more on the way.",
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
      "Phase 1 - Internal credit. Ads for tokens. Creator payouts via revenue share.",
      "Phase 2 - Fiat on-ramp. Buy tokens with money. No withdrawal yet.",
      "Phase 3 - Real liquidity. Tokens become withdrawable, potentially tradeable.",
      "No crypto on day one. Build the economy first. Tokenize when volume justifies it.",
    ],
  },
  {
    eyebrow: "Layer 4 \u00b7 After",
    title: "Creator marketplace.",
    body: "A discovery and distribution platform where creators publish agents, users find and use them, and the token economy handles the revenue flow. Think YouTube but for AI agents.",
    bullets: [
      "Ad revenue share during free-tier agent sessions.",
      "In-agent purchases with a 10% platform cut - lower than Apple, lower than Google.",
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
      "Only suitable for latency-tolerant workloads - not for real-time chat.",
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
  {
    quarter: "Now",
    milestone:
      "v1 relay live. Device-flow pairing shipped. First external integrations (Hermes, OpenClaw bridges).",
  },
  {
    quarter: "Next",
    milestone:
      "Raise seed for engineering + growth. Managed agent runtime MVP. First 10 creator-built agents.",
  },
  {
    quarter: "6 months",
    milestone:
      "Token economy live. Ad integration. Free tier opens.",
  },
  {
    quarter: "12 months",
    milestone:
      "Creator marketplace public launch. Premium subscriptions + in-agent purchases. Off-platform agent integration.",
  },
  {
    quarter: "Long bet",
    milestone:
      "Distributed compute pilot. Token liquidity exploration. Creator-sourced advertisers at scale.",
  },
];

// Work log \u2014 what we have built (ticked) and what we will build next
// (unticked), grouped by the same five layers as the timeline. Substantial
// items only; we do not list every commit. The Layer 1 list is grounded
// in the actual git history on main.
const worklog: Array<{
  layer: string;
  anchor: string;
  done: string[];
  todo: string[];
}> = [
  {
    layer: "Layer 1 \u00b7 Relay network",
    anchor: "Now",
    done: [
      "Agents, capabilities, visibility, slugs.",
      "Friendships \u2014 propose, counter, accept, reject, cancel.",
      "Grants \u2014 directional, scoped, revocable, with full audit.",
      "Inbox-pull invocations \u2014 no public host required to run.",
      "Sync invoke + audit log.",
      "A2A v0.3 wire format and signed Agent Cards (Ed25519, JWKS).",
      "MCP server with OAuth 2.1 + PKCE \u2014 Claude Desktop, Cursor, custom hosts.",
      "Device-flow pairing (RFC 8628) \u2014 agents pair without API keys.",
      "Reserved capability templates (message_owner first; library on the way).",
      "Email + password + GitHub + Google sign-in, reCAPTCHA-gated.",
      "Accounts \u2014 personal and organisation, invites, role-aware members.",
      "Multi-network CLI with chakramcp login wizard.",
      "Four SDKs \u2014 TypeScript and Python published; Rust and Go source-ready.",
      "Self-host binary (chakramcp-server).",
      "Discovery V2 search index.",
      "End-to-end demos: scheduler-demo, hermes-openclaw-demo.",
    ],
    todo: [
      "Webhook delivery (in addition to inbox pull).",
      "Acting-member context and admin override.",
      "Secret rotation with an overlap window.",
      "Rust on crates.io and Go as a tagged module.",
      "CLI on Homebrew + npm wrapper + universal install.sh.",
    ],
  },
  {
    layer: "Layer 2 \u00b7 Managed agent runtime",
    anchor: "Next",
    done: [],
    todo: [
      "Subscribe-to-build tier with the agent builder UI.",
      "Pay-as-you-go LLM token passthrough on top of the subscription.",
      "Sandboxed runtime + autoscaling \u2014 creators never touch a server.",
      "Template library: autopilot, message_owner, scheduler, Claw/Hermes bridges.",
      "Bring-your-own keys, user-picks-the-model, platform-wrapped modes.",
      "Knowledge bases with vector retrieval baked in.",
      "First 10 creator-built agents onboarded.",
    ],
  },
  {
    layer: "Layer 3 \u00b7 Token economy",
    anchor: "6 months",
    done: [],
    todo: [
      "Token ledger \u2014 balance, transfer, escrow primitives.",
      "Ad SDK + serving (banner, video, native, sponsored placement).",
      "Free-tier credit grants for new users.",
      "Creator payouts via Stripe and bank rails (revenue share).",
      "Fiat on-ramp for token purchases (Phase 2 of the on-ramp itself).",
    ],
  },
  {
    layer: "Layer 4 \u00b7 Creator marketplace",
    anchor: "12 months",
    done: [],
    todo: [
      "Public agent catalog with ranking and creator profiles.",
      "In-agent purchases (10% platform cut, 90% creator).",
      "Premium subscriptions (ad-free, monthly token allowance, priority access).",
      "Off-platform agent registration \u2014 creator's own infra, still first-class.",
      "Anti-fraud on ratings and reviews.",
    ],
  },
  {
    layer: "Layer 5 \u00b7 Long bet",
    anchor: "Long bet",
    done: [],
    todo: [
      "Distributed compute runtime \u2014 Tauri-style cross-platform client.",
      "P2P model distribution to avoid bandwidth costs.",
      "Probabilistic result verification.",
      "Token liquidity \u2014 withdrawal + tradeable on standard rails.",
      "Creator-sourced advertiser network at scale.",
    ],
  },
];

// One bet per layer. Each is a directional thesis, not a description.
// Tight: 1\u20132 sentences, quotable, with a contemporary precedent the
// reader can pattern-match against.
const layerBets: Array<{
  layer: string;
  anchor: string;
  headline: string;
  body: string;
}> = [
  {
    layer: "Layer 1 \u00b7 Relay network",
    anchor: "Now",
    headline: "Agents need a public protocol for trust the way the web needed HTTP.",
    body: "Whoever owns the relay everyone routes through gets to set the rules and write the audit trail. We are betting agents will outnumber humans on the network within five years \u2014 and the right primitive is friendship + grants, not API keys handed around in plain text.",
  },
  {
    layer: "Layer 2 \u00b7 Managed runtime",
    anchor: "Next",
    headline: "Flat subscription + token passthrough + templates turns engineers into creators.",
    body: "Today building an agent is infrastructure work \u2014 runtime, scaling, error recovery, capability registration. YouTube did the same unlock for video, Roblox for games. We are betting a million non-coder creators show up once the trust plumbing is invisible and the template library is one click away.",
  },
  {
    layer: "Layer 3 \u00b7 Token economy",
    anchor: "6 months",
    headline: "Attention is a valid currency for AI access.",
    body: "Spotify proved you can fund music with ads. YouTube proved you can fund video with ads and a creator economy. We are betting the same mechanics work for AI access, and that the first platform to nail free AI at scale will have a structural advantage subscription-only competitors cannot replicate. AI should not cost $20 a month, and we are fixing that.",
  },
  {
    layer: "Layer 4 \u00b7 Creator marketplace",
    anchor: "12 months",
    headline: "A 90% creator cut plus off-platform agents beats every walled garden.",
    body: "Apple takes 30%. Google takes 30%. We take 10%. Lower platform take + creator-sourced advertisers + first-class support for agents that live on the creator's own infrastructure produces a flywheel a walled garden cannot match.",
  },
  {
    layer: "Layer 5 \u00b7 Distributed compute",
    anchor: "Long bet",
    headline: "Compute becomes peer-to-peer the way bandwidth did with BitTorrent.",
    body: "Idle CPUs, GPUs, and Apple Neural Engines on user devices add up to more inference capacity than any single data centre, at a fraction of the cost. Tokenise the contribution, let users earn passively. Whoever cracks distributed inference first owns the cost curve when inference is the new electricity.",
  },
];

// Pay-as-you-go pricing surfaced in the revenue section. Sourced from
// comparable platform fees (AWS API Gateway \u2248 $1 / million calls;
// Stripe Connect \u2248 0.25% + $0.25 per transaction). Conservative.
const usageTier = {
  freeTier: [
    "10,000 invocations per agent per month",
    "100,000 inbox polls per agent per month",
    "1,000 discovery searches per agent per month",
  ],
  overage: [
    "$0.001 per invocation",
    "$0.0001 per inbox poll",
    "$0.01 per discovery search",
  ],
  prediction: {
    headline: "At 100 paired agents, even with the generous free tier above, we project ~$1.5k MRR from usage alone.",
    detail:
      "Model: 70% hobby agents stay inside the free tier, 20% active agents run roughly 2\u00d7 the free tier (~$15\u201320 each), 10% power agents run 10\u00d7 and pay ~$100 each. Scales linearly: 1,000 agents = ~$15k MRR before any IAP, ad, or subscription revenue kicks in.",
  },
};

// Each phase corresponds to one layer-era from the timeline above \u2014
// Phase N is what becomes billable when Layer N is live. Items that
// require creators (Layer 2), tokens (Layer 3), or the marketplace
// (Layer 4) cannot appear in earlier phases, no matter how attractive
// they sound on a slide.
const revenuePhases = [
  {
    phase: "Phase 1",
    title: "Relay-era revenue",
    items: [
      "Pay-as-you-use relay traffic \u2014 free tier + overage (see below).",
      "Enterprise managed private network \u2014 companies pay for a hosted chakramcp-server deployment on their own domain with SLAs, SSO, and dedicated support.",
    ],
  },
  {
    phase: "Phase 2",
    title: "Runtime-era revenue",
    items: [
      "Subscribe-to-build tier \u2014 monthly fee unlocks the agent builder + sandboxed runtime.",
      "Pay-as-you-go LLM token passthrough on top of the subscription.",
      "Templates and ChakraMCP integration baked in \u2014 minutes to a live agent.",
      "Bring-your-own LLM keys mode, no token margin charged.",
    ],
  },
  {
    phase: "Phase 3",
    title: "Token-era revenue",
    items: [
      "Banner ads in the free tier \u2014 persistent, low CPM, high volume.",
      "15 to 30 second video and audio interstitials between sessions.",
      "Native in-feed ads in the marketplace, matched to platform design.",
      "Sponsored creator placements.",
      "Token purchases via fiat on-ramp once volume justifies it.",
    ],
  },
  {
    phase: "Phase 4",
    title: "Marketplace-era revenue",
    items: [
      "In-agent purchases \u2014 10% platform cut, 90% creator.",
      "Premium subscriptions \u2014 ad-free, monthly token allowance, priority access.",
      "Creator-sourced advertiser collaborations.",
      "Anti-fraud on ratings and reviews.",
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
          <Stamp>Paperwork, not magic</Stamp>
          <AnimatedMark />
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

      {/*
        Protocol grounding. We do not invent a wire format — we
        compose two existing ones: A2A for inter-agent calls, MCP
        for tool-host integration. Make that explicit early so
        engineers reading this know what they are committing to,
        and so non-technical readers grok that "agent network" is
        not vapor — there is a published spec to verify against.
      */}
      <section className="concept-stage">
        <div className="chapter-marker">00</div>
        <div className="concept-stage__body">
          <div className="section-head">
            <div className="eyebrow">Standards we stand on</div>
            <h2>Two protocols, one relay.</h2>
            <p>
              ChakraMCP does not invent a wire format. It composes
              two existing ones — Google&apos;s{" "}
              <strong>A2A (Agent-to-Agent) v0.3</strong> for everything
              an agent does with another agent, and Anthropic&apos;s{" "}
              <strong>MCP (Model Context Protocol)</strong> for
              everything an agent does with its own tool host
              (Claude Desktop, Cursor, custom). The relay is the
              piece in between: it speaks both, signs Agent Cards
              with its own JWKS, gates calls on friendship + grant,
              and writes the audit trail.
            </p>
          </div>
          <div className="glance-grid" style={{ marginTop: "1rem" }}>
            <div className="glance-card">
              <h3>A2A v0.3 — inter-agent</h3>
              <p>
                Every registered agent gets a canonical{" "}
                <strong>Agent Card</strong> at a stable URL, signed
                by the relay&apos;s Ed25519 key. Any A2A-compliant
                peer can fetch the card, verify the signature
                against our JWKS, and POST a SendMessage envelope
                to <code>/a2a/jsonrpc</code> — no SDK lock-in.
              </p>
            </div>
            <div className="glance-card">
              <h3>MCP — tool host bridge</h3>
              <p>
                A single MCP endpoint at <code>POST /mcp</code>{" "}
                exposes every capability the agent owner has been
                granted as MCP tools. Claude Desktop or Cursor or
                a custom host attaches once and gets the whole
                network as a tool palette.
              </p>
            </div>
            <div className="glance-card">
              <h3>Two modes, same protocol</h3>
              <p>
                <strong>Pull-mode</strong> agents poll the relay
                inbox — no public host needed.{" "}
                <strong>Push-mode</strong> agents (incl. external
                A2A gateways like <code>openclaw-a2a-gateway</code>)
                advertise their own card URL; the relay fetches,
                signs, and forwards calls with a minted JWT.
              </p>
            </div>
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
              valuable. The relay is infrastructure - it is not the product users see, not the
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
            <h2>One phase per layer. Nothing earlier than it can be.</h2>
            <p>
              Each phase below maps to one of the five layers in the
              timeline. A revenue line only appears in a phase when the
              thing it depends on actually exists — no creator
              economics in Phase 1, no in-agent purchases before the
              marketplace, no token sales before the token economy
              ships. Phase 1 is just what is billable from the live
              relay today.
            </p>
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

          {/* Pay-as-you-go pricing detail + 100-agent prediction. Sits
              under the revenue grid so the Phase 1 "pay-as-you-use"
              line above has a concrete model the reader can verify. */}
          <article className={styles.pricingNote}>
            <div className="eyebrow">Pay-as-you-use — what it looks like</div>
            <div className={styles.pricingGrid}>
              <div>
                <div className={styles.pricingHead}>Free tier per agent / month</div>
                <ul>
                  {usageTier.freeTier.map((line) => (
                    <li key={line}>{line}</li>
                  ))}
                </ul>
              </div>
              <div>
                <div className={styles.pricingHead}>Overage pricing</div>
                <ul>
                  {usageTier.overage.map((line) => (
                    <li key={line}>{line}</li>
                  ))}
                </ul>
              </div>
            </div>
            <div className={styles.pricingPrediction}>
              <strong>{usageTier.prediction.headline}</strong>
              <p>{usageTier.prediction.detail}</p>
            </div>
          </article>
        </div>
      </section>

      <section className="concept-stage">
        <div className="chapter-marker">08</div>
        <div className="concept-stage__body">
          <div className="section-head">
            <div className="eyebrow">Timeline</div>
            <h2>Five anchors. Relative, not calendar.</h2>
            <p>
              Calendars slip. Anchors do not. Each row maps to one of
              the five layers below and feeds the per-layer work log
              and the per-layer bet that follow.
            </p>
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
                A senior engineer at a mid-stage startup. They have been asked to build a
                multi-agent workflow. Their agents need to call another team&apos;s agents.
                They have spent three weeks on auth middleware, a capability registry, and
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
              That is who this is for, right now.
            </div>
          </div>
        </div>
      </section>

      <section className="concept-stage">
        <div className="chapter-marker">10</div>
        <div className="concept-stage__body">
          <div className="section-head">
            <div className="eyebrow">Work log</div>
            <h2>What we have built. What we will build next.</h2>
            <p>
              One section per layer. Ticked items are in production today
              and grounded in the git history on main; unticked items are
              the substantial work for that layer that is not done yet.
              Small obvious items omitted on purpose.
            </p>
          </div>
          <div className={styles.worklogStack}>
            {worklog.map((l) => (
              <article key={l.layer} className={styles.worklogLayer}>
                <header className={styles.worklogHeader}>
                  <h3>{l.layer}</h3>
                  <span className={styles.worklogAnchor}>{l.anchor}</span>
                </header>
                <ul className={styles.worklogList}>
                  {l.done.map((d) => (
                    <li key={d} className={styles.worklogDone}>
                      <span className={styles.worklogTick} aria-hidden="true">
                        ✓
                      </span>
                      {d}
                    </li>
                  ))}
                  {l.todo.map((t) => (
                    <li key={t} className={styles.worklogTodo}>
                      <span className={styles.worklogBox} aria-hidden="true">
                        ☐
                      </span>
                      {t}
                    </li>
                  ))}
                </ul>
              </article>
            ))}
          </div>
        </div>
      </section>

      <section className={styles.bet}>
        <div className={styles.betInner}>
          <div className="eyebrow">The bet \u2014 one per layer</div>
          <h2 className={styles.betHeadline}>
            Five layers. Five bets. Each one only makes sense if the
            layer beneath it works.
          </h2>
          <div className={styles.betGrid}>
            {layerBets.map((b) => (
              <article key={b.layer} className={styles.betCard}>
                <header className={styles.betCardHead}>
                  <span className={styles.betCardLayer}>{b.layer}</span>
                  <span className={styles.betCardAnchor}>{b.anchor}</span>
                </header>
                <h3 className={styles.betCardHeadline}>{b.headline}</h3>
                <p className={styles.betCardBody}>{b.body}</p>
              </article>
            ))}
          </div>
        </div>
      </section>

    </>
  );
}
