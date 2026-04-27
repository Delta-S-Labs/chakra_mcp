import type { Metadata } from "next";
import Link from "next/link";
import styles from "./cofounder.module.css";

export const metadata: Metadata = {
  title: "Cofounder \u2014 ChakraMCP",
  description:
    "I'm building ChakraMCP, a relay network for AI agents. Open source for self-hosts, a managed public network for the rest. Looking for one technical cofounder.",
  robots: { index: false, follow: false },
};

const shipped = [
  {
    label: "Marketing site live",
    detail: "Portfolio, concept, brand. Four portfolio examples end-to-end. Auto-deploys on merge to main.",
    href: "https://chakra-mcp.netlify.app",
  },
  {
    label: "Public GitHub",
    detail: "Full source \u2014 frontend, CI, render pipeline, design system, build spec. Clone it, read it.",
    href: "https://github.com/Delta-S-Labs/chakra_mcp",
  },
  {
    label: "Design system",
    detail: "Tokens, type, color, logo, CSS primitives. Packaged as a Claude Code skill so anyone can generate ChakraMCP-branded UI.",
  },
  {
    label: "CI green on every push",
    detail: "Lint + typecheck + build, CodeQL security scan, dependency audit, Dependabot grouped updates.",
  },
  {
    label: "Offline render pipeline",
    detail: "Playwright + ffmpeg renders MP4 / GIF of the animated dispatch-log example for social share. Deterministic, frame-perfect.",
  },
  {
    label: "Full backend spec",
    detail: "Data model (11 tables), API surface (~30 endpoints), phased build order, AWS deploy shape. In docs/chakramcp-build-spec.md.",
  },
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

const risks = [
  {
    title: "The market is 12\u201318 months early.",
    body: "Most \u201Cagents\u201D today are single-tenant chatbots with system prompts. The multi-agent workflows this relay is built for are coming, not arrived. We\u2019re betting first-mover compounds into defensibility before the market catches up.",
  },
  {
    title: "Incumbents are converging.",
    body: "Anthropic has Claude Managed Agents. OpenAI has the Agents SDK. Google has Agent Space. None of them solve networked trust across organizations, but any of them could build a relay layer in six months if they decide to. Our bet is that being neutral and cross-platform outlasts being vertical.",
  },
  {
    title: "Usage-based pricing creates friction.",
    body: "Charging per relay call is a cost layer on top of free protocols (raw MCP, WebSockets, HTTP). The free tier has to be generous enough that developers complete their first integration without noticing. That\u2019s on us.",
  },
];

const culture = [
  {
    title: "Everything written.",
    body: "Decisions happen in the repo, in tickets, in docs \u2014 not in a Slack thread that scrolls away. If it\u2019s not written, it didn\u2019t happen.",
  },
  {
    title: "Low process, high ownership.",
    body: "No standups-for-standups, no weekly status rituals, no approvals-of-approvals. You pick up a thing. You ship it. You say what you learned.",
  },
  {
    title: "Engineers talk to users.",
    body: "Directly. Not through a PM. Not through a support rep. Not through a dashboard. The person who built it explains it to the person using it.",
  },
  {
    title: "Open defaults.",
    body: "Public repo. Public roadmap (once there\u2019s one worth reading). Public decisions when we can swing it. The only things that stay internal are the things that actually have to.",
  },
  {
    title: "Warm, blunt, unserious.",
    body: "The brand is editorial-zine, not enterprise-deck. The voice is the same inside the team. We say what we think. We laugh at ourselves. We don\u2019t pretend to be a serious big company when we\u2019re clearly not one yet.",
  },
  {
    title: "Boring infrastructure, exciting product.",
    body: "Shipping a reliable relay is boring work done well. The product we build on top of it is where the excitement lives. You should like both.",
  },
];

export default function CofounderPage() {
  return (
    <>
      <section className={styles.hero}>
        <div className={styles.heroInner}>
          <div className="eyebrow">Cofounder page</div>
          <h1 className={styles.heroHeadline}>Come build this with me.</h1>
          <p className={styles.heroLead}>
            ChakraMCP is a relay network for AI agents &mdash; discovery, friendship, directional
            grants, consent, audit. Open source for anyone who wants to self-host (an internal
            company network, a private deployment, anywhere). A managed public network for
            everyone who doesn&apos;t. Free to host yourself; usage-based on the public network,
            with a generous free tier at launch. I&apos;m building it solo. I&apos;m looking for
            one technical cofounder who likes boring, excellent infrastructure.
          </p>
          <div className={styles.heroLinks}>
            <a className="pill-link" href="https://github.com/Delta-S-Labs/chakra_mcp" target="_blank" rel="noreferrer">
              See the repo
            </a>
          </div>
        </div>
      </section>

      <section className={styles.stage}>
        <div className={styles.stageMark}>01</div>
        <div className={styles.stageBody}>
          <div className="eyebrow">What this is</div>
          <h2>Social infrastructure for software that acts on behalf of people.</h2>
          <p>
            Agents discover other agents. Some handshakes turn into friendships. Some friendships
            unlock the ability to run each other&apos;s tools &mdash; always with consent, always
            with audit. The protocol spec, data model, and full roadmap live on the concept page.
            This page is for the part of the pitch that concerns you if you want to build it with
            me.
          </p>
        </div>
      </section>

      <section className={styles.stage}>
        <div className={styles.stageMark}>02</div>
        <div className={styles.stageBody}>
          <div className="eyebrow">Who this is for, right now</div>
          <h2>Two kinds of early users.</h2>
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
            <div className={styles.portraitFoot}>That&apos;s who this is for, right now.</div>
          </div>
        </div>
      </section>

      <section className={styles.stage}>
        <div className={styles.stageMark}>03</div>
        <div className={styles.stageBody}>
          <div className="eyebrow">What&apos;s shipped</div>
          <h2>Concrete things you can verify in twenty minutes.</h2>
          <ul className={styles.shippedList}>
            {shipped.map((s) => (
              <li key={s.label}>
                <div className={styles.shippedLabel}>
                  {s.href ? (
                    <a href={s.href} target="_blank" rel="noreferrer">
                      {s.label}
                    </a>
                  ) : (
                    s.label
                  )}
                </div>
                <div className={styles.shippedDetail}>{s.detail}</div>
              </li>
            ))}
          </ul>
        </div>
      </section>

      <section className={styles.profile}>
        <div className={styles.profileMark} aria-hidden="true">
          <span className={styles.profileDot} />
        </div>
        <div className={styles.stageBody}>
          <div className="eyebrow">Why me</div>
          <h2>Why I&apos;m building this.</h2>
          <p>
            I&apos;ve built systems that scale. I&apos;ve built AI agents that solve real-world
            problems. That&apos;s the credentials part. It&apos;s not what got me here.
          </p>
          <p>
            What got me here: every time I needed two agents to actually talk to each other, the
            answer was either chain them inside one framework, or hand-write an MCP that exposes
            what one agent will let another agent run. There&apos;s no network for that. Just
            teams reinventing the trust layer in private, slightly differently each time. I
            wanted to find one that solved our problems. There wasn&apos;t one.
          </p>
          <p>
            The future workplace looks different to me. People will bring their own specialized
            agents to work &mdash; running locally, running on a trusted cloud, wherever they
            keep them. Research agents, code agents, calendar agents, ones nobody&apos;s thought
            of yet. They&apos;ll need to find each other across teams, across companies, across
            personal-and-work boundaries. They&apos;ll need a communication protocol that
            handles trust without forcing everyone into a single vendor&apos;s runtime.
          </p>
          <p>
            ChakraMCP is a start at that. Today it&apos;s a relay. It might evolve into a
            protocol. The wire is HTTP for v1; gRPC or SSE later if the latency or streaming
            shape demands it. The point is the network, not the transport.
          </p>
          <div className={styles.profileLink}>
            <a href="https://banerjee.life" target="_blank" rel="noreferrer">
              Longer version → banerjee.life
            </a>
          </div>
        </div>
      </section>

      <section className={styles.stage}>
        <div className={styles.stageMark}>04</div>
        <div className={styles.stageBody}>
          <div className="eyebrow">Tempo</div>
          <h2>Where we are. Where we&apos;re going in the next two weeks.</h2>
          <p>
            Frontend is shipped and iterating. Backend spec is written; code is not yet. The next
            two weeks are Phase 1 of the Rust backend &mdash; Axum server, Postgres schema, agent
            registration + discovery + auth, health checks. After that: access requests, grants,
            and first relay call end-to-end. You&apos;d join mid-flight. The spec is done. The
            tooling is set up. The marketing site is already live. What&apos;s left is building
            the relay and finding the first ten users who use it instead of writing the trust
            layer themselves.
          </p>
        </div>
      </section>

      <section className={styles.stage}>
        <div className={styles.stageMark}>05</div>
        <div className={styles.stageBody}>
          <div className="eyebrow">What ships first vs. later</div>
          <h2>Six capabilities. Everything else is v2.</h2>
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

      <section className={styles.stage}>
        <div className={styles.stageMark}>06</div>
        <div className={styles.stageBody}>
          <div className="eyebrow">Where this is hard</div>
          <h2>Three risks that keep me honest.</h2>
          <div className={styles.risks}>
            {risks.map((r) => (
              <article key={r.title} className={styles.risk}>
                <h3>{r.title}</h3>
                <p>{r.body}</p>
              </article>
            ))}
          </div>
        </div>
      </section>

      <section className={styles.stage}>
        <div className={styles.stageMark}>07</div>
        <div className={styles.stageBody}>
          <div className="eyebrow">What we&apos;re up against</div>
          <h2>The real enemy is custom code.</h2>
          <p>
            Every team building multi-agent systems writes their own auth middleware, their own
            capability registry, their own consent flow, their own audit trail. It works. It&apos;s
            ugly. Every team does it slightly differently. The relay replaces all of that &mdash;
            but only if switching is cheaper than maintaining what they&apos;ve already built.
          </p>
          <p>
            Adjacent threats: single-tenant agent runtimes from the big labs that solve
            orchestration inside their walls but not across trust boundaries; local orchestration
            frameworks (LangGraph, CrewAI, AutoGen) that work inside one deployment; API gateways
            that don&apos;t know what an agent is. None of them have built a networked trust
            layer. Any of them could if the market asks loudly enough.
          </p>
        </div>
      </section>

      <section className={styles.stage}>
        <div className={styles.stageMark}>08</div>
        <div className={styles.stageBody}>
          <div className="eyebrow">Tech stack</div>
          <h2>Rust in the back, TypeScript in the front, AWS and Netlify underneath.</h2>
          <div className={styles.stackGrid}>
            <article className={styles.stackBlock}>
              <div className={styles.stackLabel}>Backend (in progress)</div>
              <ul>
                <li>Rust + Axum + Tokio</li>
                <li>Postgres via sqlx (compile-time checked queries)</li>
                <li>JWT for API auth, HMAC-SHA256 for signed webhooks</li>
                <li>AWS: ECS Fargate + RDS Postgres + ALB + Secrets Manager</li>
                <li>Containerized, portable, standard 12-factor config</li>
              </ul>
            </article>
            <article className={styles.stackBlock}>
              <div className={styles.stackLabel}>Frontend (shipped)</div>
              <ul>
                <li>Next.js 16 App Router, React 19, TypeScript</li>
                <li>CSS modules + design tokens (no Tailwind)</li>
                <li>motion/react for scroll &amp; state animations</li>
                <li>Netlify via the Next.js plugin</li>
              </ul>
            </article>
            <article className={styles.stackBlock}>
              <div className={styles.stackLabel}>Tooling</div>
              <ul>
                <li>GitHub Actions: lint, build, CodeQL, dep audit, Dependabot</li>
                <li>Playwright + ffmpeg for offline animation render</li>
                <li>pnpm everywhere</li>
              </ul>
            </article>
            <article className={styles.stackBlock}>
              <div className={styles.stackLabel}>Docs</div>
              <ul>
                <li>
                  <code>docs/chakramcp-build-spec.md</code> &mdash; full backend spec
                </li>
                <li>
                  <code>docs/chakramcp-investor-roadmap.md</code> &mdash; platform vision
                </li>
                <li>
                  <code>docs/ChakraMCP Design System/</code> &mdash; tokens, UI kits, SKILL.md
                </li>
              </ul>
            </article>
          </div>
        </div>
      </section>

      <section className={styles.stage}>
        <div className={styles.stageMark}>09</div>
        <div className={styles.stageBody}>
          <div className="eyebrow">What I&apos;m looking for</div>
          <h2>How we work, not how many hours.</h2>
          <ul className={styles.workingList}>
            <li>
              <strong>Committed development.</strong> If you start something, you finish it. You
              own what you build &mdash; the design, the deploy, the on-call, the feedback loop
              with users.
            </li>
            <li>
              <strong>Shipping speed.</strong> Boring infrastructure ships boring PRs quickly, not
              perfect ones slowly. Small batches. Merge early. Iterate in public. No 30-page RFCs
              for things we could just build.
            </li>
            <li>
              <strong>Comfortable with the open-source split.</strong> The relay will be open
              source &mdash; anyone can self-host it inside a company, inside a private network,
              wherever they want. We run the public network as a managed service. You should be at
              home contributing to a codebase anyone can fork while we build a hosted product on
              top of it.
            </li>
          </ul>
        </div>
      </section>

      <section className={styles.stage}>
        <div className={styles.stageMark}>10</div>
        <div className={styles.stageBody}>
          <div className="eyebrow">Culture</div>
          <h2>How the inside feels.</h2>
          <div className={styles.cultureGrid}>
            {culture.map((c) => (
              <article key={c.title} className={styles.cultureCard}>
                <h3>{c.title}</h3>
                <p>{c.body}</p>
              </article>
            ))}
          </div>
        </div>
      </section>

      <section className={styles.ask}>
        <div className={styles.askInner}>
          <div className="eyebrow">The ask</div>
          <h2 className={styles.askHeadline}>
            Email me <a href="mailto:kaustav@banerjee.life">kaustav@banerjee.life</a>.
          </h2>
          <p className={styles.askBody}>
            Include your background, one thing you&apos;d want to change about the plan on this
            page, and one thing you&apos;d build first. I read everything.
          </p>
        </div>
      </section>
    </>
  );
}
