# What Comes After ChakraMCP Goes Live

**A roadmap for investors: the components we build on top of the relay network, and why each one compounds the last.**

---

## Where We Are Today

ChakraMCP is a relay network for AI agents. Agents register, publish capabilities, negotiate trust through friendship and directional grants, and execute remote tools and workflows through a managed relay — all with consent-aware access control and full audit trails. It's written in Rust, deployed on AWS, and designed to be the trust and communication layer for an agent economy that doesn't exist yet.

The network itself is infrastructure. It is not the product users see, not the business model, and not the thing that generates revenue. It's the foundation that makes everything below possible — the same way TCP/IP is the foundation of the internet but nobody pays for TCP/IP.

Here's what we build on top of it, in what order, and why.

---

## The Full Platform Stack

```
┌─────────────────────────────────────────────────────────┐
│  5. Distributed Compute Network (device resource renting)│
├─────────────────────────────────────────────────────────┤
│  4. Creator Marketplace + Revenue Sharing                │
├─────────────────────────────────────────────────────────┤
│  3. Token Economy (earn/spend/buy/withdraw)              │
├─────────────────────────────────────────────────────────┤
│  2. Managed Agent Runtime (build & deploy agents)        │
├─────────────────────────────────────────────────────────┤
│  1. ChakraMCP Relay Network (trust & communication)      │  ← WE ARE HERE
└─────────────────────────────────────────────────────────┘
```

Each layer depends on the one below it. Each layer makes the ones above it more valuable. The network is the spine. Everything else is muscle.

---

## Layer 2: Managed Agent Runtime

### What It Is

A platform where creators define AI agents and we handle everything else — the runtime, the infrastructure, the sandboxed execution environment, session continuity, error recovery, scaling. Creators never touch a server, never configure a container, never write an agent loop. They describe the agent, configure its tools and knowledge, set its guardrails, and publish. We run it.

This is the same concept as Anthropic's Claude Managed Agents — a fully managed cloud environment where agents can execute code, browse the web, handle files, and call external services — except we're building it for consumer creators funded by ads, not enterprises paying $0.08/session-hour.

### What Creators Configure

**The agent itself.** What it does, how it behaves, what its personality is. Natural language or structured configuration. Guardrails: what the agent can and can't do, what topics it stays within, what actions require user confirmation.

**Knowledge bases.** Documents, datasets, domain expertise. The immigration lawyer uploads every visa guideline. The fitness coach uploads their programming methodology. The agent becomes a distillation of real domain knowledge.

**Tools and MCPs.** Three tiers:

- **Platform-provided:** Web search, code execution, file handling, image generation. Toggle on, no API keys needed.
- **Creator-authenticated:** The creator connects their own service credentials. A recruiter connects their ATS. A real estate agent connects their MLS.
- **User-authenticated:** Integrations that need the user's credentials. Google Calendar, Spotify, bank accounts. The user authenticates once, scoped access is managed by the platform.

**LLM configuration.** Three modes:

- **Creator-provided LLM:** Bring your own API keys. You pay for compute, you control the model.
- **User's chosen LLM:** The agent uses whatever model the user selects. The user spends their own tokens. Creator pays nothing for compute.
- **Platform-wrapped LLM:** We provide the infrastructure, handle provider routing, deal with rate limits. Simplest option.

**Off-platform agents.** Agents that live entirely on the creator's infrastructure, use their own LLM, but connect to ChakraMCP's user base and token economy through the relay network. Creators who outgrow the managed runtime don't leave — they graduate to self-hosted agents that still participate in the marketplace.

### Why It Matters

Without the managed runtime, building an agent that actually does things (executes code, browses the web, handles files, calls APIs) requires months of infrastructure work. The managed runtime turns "months to first agent" into "afternoon to first agent." That's what creates the supply side of the marketplace.

### Technical Approach

- Sandboxed execution via containerized environments (Firecracker microVMs or Fly Machines)
- Session state persisted to PostgreSQL, keyed by agent + user
- MCP integration through ChakraMCP relay (agents on the platform are first-class network citizens)
- Knowledge base storage in object storage (S3-compatible) with vector indexing for retrieval
- Background job processing via the same event system used by ChakraMCP relay

### Dependencies

Requires ChakraMCP relay network (Layer 1) for agent-to-agent communication, capability registration, and trust management.

---

## Layer 3: Token Economy

### What It Is

A universal platform currency that flows between every participant in the system. Every economic action — earning, spending, building, using — goes through the same token.

### How Tokens Are Earned

**Watching ads.** The primary earning mechanism at launch. Users watch 15-30 second video/audio ads or see banner ads, earning tokens proportional to ad value. The exchange rate is tuned so the friction is Spotify-level: noticeable, mildly annoying, and eventually converting some percentage of users to premium.

**Renting device compute (future).** Users contribute idle CPU/memory/GPU to run small local LLMs for the platform. Tokens earned passively when devices are idle. This is the "mining" equivalent, except it produces actual useful work instead of burning electricity on proof-of-work.

**Buying with money.** Premium subscriptions include monthly token allowances. Pay-as-you-go option for users who want more tokens without a subscription.

### How Tokens Are Spent

**AI usage.** Every query, every agent interaction, every model call costs tokens. Different models cost different amounts. Using a creator's agent costs tokens that flow partly to the creator.

**In-agent purchases (future).** Creators can sell premium features inside their agents. The fitness coach charges tokens for a personalized training plan. The legal advisor charges per document review.

### How Creators Earn

Creators accumulate tokens proportional to their agent's usage. When users interact with a creator's agent, the tokens spent flow partly to the creator. Creators cash out through a traditional payout program — once they cross a threshold, we convert accumulated tokens to real money via standard payment rails (Stripe, bank transfer).

### Phase Evolution

**Phase 1 (Launch):** Internal credit system. Earn by watching ads, spend on AI. Creator payouts through revenue share program. No withdrawal to real money for users — tokens are a virtual good, not a currency. This avoids money transmitter regulation entirely.

**Phase 2 (Post-traction):** Fiat on-ramp. Users can buy tokens with real money. Still no withdrawal. This is an in-app purchase, well-trodden legal ground.

**Phase 3 (Post-scale):** Real liquidity. Tokens become withdrawable, potentially tradeable, potentially on-chain. This requires regulatory compliance infrastructure (KYC/AML, money transmitter licenses) and is only worth pursuing when volume justifies the investment.

### Why Not Crypto From Day 1

The "AI + crypto" pitch attracts speculators instead of AI users, crypto tourists instead of product-focused investors, and SEC inquiries before there's revenue. Build the economy first. Make the token valuable through utility. Tokenize when the economy is real. Spotify didn't launch as a crypto project.

### Technical Approach

- Token ledger as a PostgreSQL table with double-entry bookkeeping
- Transaction types: ad_reward, purchase, usage_debit, creator_credit, payout, device_rent_reward
- Idempotent transaction processing (every token movement has a unique transaction ID)
- Real-time balance queries with materialized running totals
- Ad integration via standard ad networks initially (Google AdMob, Meta Audience Network), direct sponsor deals as volume grows

### Dependencies

Requires managed agent runtime (Layer 2) for token metering on agent usage. Requires ChakraMCP relay (Layer 1) for cross-agent token attribution.

---

## Layer 4: Creator Marketplace

### What It Is

A discovery and distribution platform where creators publish agents, users find and use them, and the token economy handles the revenue flow. Think YouTube but for AI agents — except instead of uploading a video, you're publishing an agent that solves real problems.

### What Users See

A searchable catalog of agents organized by category, use case, and quality signals. Each agent has a profile: what it does, what it's good at, who built it, usage stats, user ratings. Users can try agents with their earned tokens, bookmark favorites, and share discoveries.

### What Creators See

An analytics dashboard showing agent usage, token earnings, user engagement, and conversion metrics. Tools for managing agent versions, responding to user feedback, and optimizing agent performance.

### Revenue Streams for Creators

**Ad revenue share (launch).** When users interact with a creator's agent, the ads shown during those sessions generate revenue. Creators get a proportional cut.

**In-agent purchases (future).** Creators can sell premium features, content, or capabilities inside their agents. Platform takes a 10% cut. Creator keeps 90%.

**Creator-sourced advertisers (future).** Creators with brand relationships bring advertisers directly onto the platform as collaborators. The fitness coach brings a supplement brand. The coding tutor brings a developer tools company. Creators earn a premium on these placements because they're brokering a direct audience match.

### Why 10% Take Rate

Apple charges 30%. Google charges 30%. Even "creator-friendly" platforms charge 15-20%. At 10%, we're making a deliberate bet that lower take rates attract more creators, more creators attract more users, and volume compensates for margin. This is the Amazon AWS playbook applied to a creator economy.

### Technical Approach

- Agent registry built on ChakraMCP's existing agent and capability catalog
- Search and discovery via PostgreSQL full-text search initially, Meilisearch or Typesense later
- User ratings and reviews stored per agent, with anti-fraud protections
- Creator analytics pipeline processing token transaction events
- Featured/sponsored placement system for marketplace monetization

### Dependencies

Requires token economy (Layer 3) for revenue flow. Requires managed agent runtime (Layer 2) for hosted agents. Requires ChakraMCP relay (Layer 1) for off-platform agent integration.

---

## Layer 5: Distributed Compute Network

### What It Is

A decentralized compute layer where users contribute idle device resources (CPU, memory, GPU) to run small local LLMs for the platform. Users earn tokens passively when their devices are idle. The platform gets cheaper inference. Everyone wins.

### Why This Matters

LLM inference is the single largest cost line for any AI platform. If we can offload a percentage of inference to user-contributed devices, we fundamentally change the cost structure. Even shifting 10-20% of small-model inference to distributed compute reduces infrastructure costs meaningfully and creates a new earning mechanism that doesn't depend on ad inventory.

### How It Works

1. User installs a lightweight agent on their device (desktop app, browser extension, or mobile background process)
2. When the device is idle (user-configurable thresholds), the agent accepts inference jobs from the platform
3. The agent runs small quantized models (Llama 3 8B, Phi-3, Mistral 7B) on local hardware
4. Results are verified and returned to the requesting agent/user
5. The device owner earns tokens proportional to compute contributed

### What Makes This Hard

- **Heterogeneous hardware.** Consumer devices vary wildly in capability. Need dynamic job routing based on device specs.
- **Reliability.** Devices go offline mid-inference. Need redundant job assignment and result verification.
- **Security.** Running arbitrary inference on user devices requires careful sandboxing. Model weights need protection against extraction.
- **Latency.** Local inference on consumer hardware is slower than cloud GPUs. Only suitable for latency-tolerant workloads.
- **Result verification.** How do you know the device actually ran the model correctly? Need lightweight verification mechanisms.

### Why Not Now

This is a second company worth of engineering. Projects like BOINC, Folding@Home, Petals, and Together have spent years on distributed compute. It's not unsolvable, but it's a distraction during the phase where we need to prove the core flywheel works. Build this after the ad-supported model is validated and the creator marketplace has traction.

### Technical Approach (Future)

- Lightweight inference runtime distributed as a cross-platform app (Tauri or Electron)
- Model distribution via torrent-style P2P to avoid bandwidth costs
- Job queue managed by the platform, routed based on device capability profiles
- Result verification via probabilistic checking (run same job on 2 devices, compare outputs)
- Token rewards calculated based on compute time, model size, and result quality

### Dependencies

Requires token economy (Layer 3) for reward distribution. Requires platform user base (all lower layers) for device supply.

---

## The Flywheel

Here's why the layers compound:

**More users → more ad revenue → more creator payouts → more creators build agents → more reasons to use the platform → more users.**

But the distributed compute layer adds a second flywheel:

**More users → more device compute available → cheaper inference → better economics → lower barrier for creators → more agents → more users.**

And the off-platform agent integration adds a third:

**More users on the platform → more external agents connect to reach the audience → bigger agent catalog → more reasons for users to stay → more users.**

Three interlocking flywheels. Each one accelerates the others. The relay network is the axis all three spin on.

---

## Competitive Landscape

| Player | What They Do | What They Don't Do |
|---|---|---|
| **OpenAI (ChatGPT)** | Best consumer AI product | No creator economy, no ad-supported tier, no agent marketplace |
| **Anthropic (Claude)** | Best enterprise agent infra | Enterprise-only pricing, no consumer play, no creator revenue |
| **Google (Gemini)** | Multi-modal AI, massive distribution | No creator economy, no agent marketplace, ad model is search-based not attention-based |
| **Hugging Face** | Model hosting, developer community | Developer-only, no consumer distribution, no business model for non-technical creators |
| **GPT Store** | Agent marketplace concept | No revenue for creators, no managed runtime, no ad-supported access |
| **Poe** | Multi-model access | Subscription-only, limited creator tools, no ad tier |
| **ChakraMCP (Us)** | Ad-supported multi-model AI + managed agent runtime + creator marketplace + relay network | We haven't built it yet |

The gap: nobody is building a consumer-facing AI platform with a Spotify-style freemium model, a YouTube-style creator economy, and enterprise-grade agent infrastructure accessible to non-developers.

---

## Revenue Model

### Revenue Streams (Phased)

**Phase 1 — Ad Revenue**
- Banner ads in the free tier (persistent, low CPM, high volume)
- Video/audio interstitials (15-30 seconds, higher CPM, between sessions)
- Native in-feed ads in marketplace (matched to platform design)
- Sponsored creator placements (promoted agents)
- Anonymized audience data licensing (intent-based segments, no PII)

**Phase 2 — Premium Subscriptions**
- Ad-free experience with monthly token allowance
- Priority access to popular creators
- Higher usage limits
- Exclusive features

**Phase 3 — Platform Economics**
- In-agent purchases (10% platform cut)
- Creator-sourced advertiser collaborations
- Token purchases (fiat on-ramp)
- Enterprise API access for high-volume integrators

**Phase 4 — Compute Economics**
- Spread between token cost to users and compute cost from distributed devices
- Premium inference tiers (faster models, guaranteed latency)

### Unit Economics (Conservative Estimates)

| Metric | Value |
|---|---|
| Ad revenue per DAU (display + video) | $2-5/month |
| AI inference cost per DAU | $1-3/month |
| Premium conversion rate | 5-10% |
| Premium ARPU | $15/month |
| Blended ARPU (free + premium) | $3-6/month |
| Creator payout percentage | 40-55% of agent-attributed revenue |
| In-agent purchase take rate | 10% |

The margin exists at moderate scale. The margin becomes compelling at scale because ad CPMs increase with targeting quality, inference costs decrease with distributed compute, and premium conversion compounds with product quality.

---

## Funding Strategy

### Current Stage

Bootstrapping. Building the ChakraMCP relay network. No external funding.

### Seed Round Trigger

Raise when we have:
- ChakraMCP relay network live and handling real agent traffic
- Managed agent runtime operational with 10+ creator-built agents
- 500+ DAU on the free tier watching ads for tokens
- Evidence that users actually watch ads for AI access (the core thesis)

### What Seed Funds

- Engineering team (3-4 engineers: Rust backend, React frontend, infra)
- Initial ad integration and direct sponsor deals
- Creator onboarding and early marketplace
- 12-18 months runway

### Series A Trigger

Raise when the flywheel is visibly spinning:
- 50K+ MAU
- 100+ active creator agents
- Measurable premium conversion rate
- Positive unit economics at current scale

---

## Timeline

| Quarter | Milestone |
|---|---|
| **Q2 2026** | ChakraMCP relay network v1 live (Rust, AWS) |
| **Q3 2026** | Managed agent runtime MVP. First 10 creator agents. |
| **Q4 2026** | Token economy live. Ad integration. Free tier opens. |
| **Q1 2027** | Creator marketplace. Public launch. Seed fundraise. |
| **Q2-Q3 2027** | Premium subscriptions. In-agent purchases. Scale creators. |
| **Q4 2027** | Creator-sourced advertisers. Off-platform agent integration. |
| **2028** | Distributed compute pilot. Token liquidity exploration. |

---

## The Bet

We're betting that **attention is a valid currency for AI access**, the same way it funds music (Spotify), video (YouTube), and news (most of the internet). We're betting that a creator economy can emerge around AI agents the way it emerged around videos, podcasts, and newsletters. And we're betting that the first platform to nail free AI access at scale will have a structural advantage that subscriptions-only competitors can't replicate.

ChakraMCP is the foundation. It's the trust and communication layer that makes agent-to-agent collaboration possible without a human babysitting every handshake. Everything we build on top of it — the managed runtime, the token economy, the creator marketplace, the distributed compute network — is in service of one idea:

**AI shouldn't cost $20/month. We're fixing that.**

---

*If we're right, we're building the access layer to AI for the entire internet. If we're wrong, we've built a working agent relay network with a weird ad experiment bolted on. The downside is survivable. The upside is generational.*
