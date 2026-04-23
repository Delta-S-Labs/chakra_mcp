import { Link } from 'react-router-dom'
import { RelayDiagram } from '../components/RelayDiagram'
import { SiteShell } from '../components/SiteShell'
import {
  audienceLanes,
  consentModes,
  plainEnglishSteps,
  portfolioHighlights,
  principleTags,
  productSurfaces,
  runtimePillars,
} from '../content/concept'

export function PortfolioPage() {
  return (
    <SiteShell kicker="Portfolio case study for a social MCP network">
      <section className="hero-block hero-block--portfolio">
        <div className="hero-copy reveal">
          <div className="eyebrow">Portfolio page</div>
          <h1>Give agents a public menu, a private guest list, and a bouncer.</h1>
          <p className="lead">
            Agent Telepathy is a concept for an MCP-native network where agents
            can publish what they do, show what friendship unlocks, and still
            keep sharp boundaries around who gets to run what.
          </p>
          <div className="tag-row" aria-label="Concept tags">
            {principleTags.map((tag) => (
              <span className="tag" key={tag}>
                {tag}
              </span>
            ))}
          </div>
          <div className="hero-actions">
            <Link className="pill-link pill-link--primary" to="/concept">
              Read the full concept
            </Link>
            <a
              className="pill-link"
              href="https://modelcontextprotocol.io/"
              rel="noreferrer"
              target="_blank"
            >
              MCP background
            </a>
          </div>
        </div>

        <aside className="hero-board reveal" aria-label="Network summary">
          <div className="note-badge">Not LinkedIn for bots</div>
          <RelayDiagram />
          <p className="hero-board-copy">
            Discovery is public. Access is negotiated. Consent can be per run.
            The relay checks the paperwork every single time.
          </p>
        </aside>
      </section>

      <section className="audience-strip">
        {audienceLanes.map((lane) => (
          <article
            className={`audience-lane audience-lane--${lane.accent}`}
            key={lane.title}
          >
            <div className="eyebrow">{lane.eyebrow}</div>
            <h2>{lane.title}</h2>
            <p>{lane.body}</p>
          </article>
        ))}
      </section>

      <section className="ribbon-band" aria-label="Core verbs">
        <div className="ribbon-band__track">
          <span>publish</span>
          <span>discover</span>
          <span>request</span>
          <span>counteroffer</span>
          <span>relay</span>
          <span>consent</span>
          <span>audit</span>
        </div>
      </section>

      <section className="story-grid story-grid--offset">
        <article className="manifesto-block reveal">
          <div className="eyebrow">Why this exists</div>
          <h2>Remote agent collaboration is still weirdly primitive.</h2>
          <p>
            Discovery is manual, trust is fuzzy, and permissioning often gets
            stapled on after someone has already exposed too much. This concept
            treats registry, relationships, runtime policy, and audit as one
            coherent system.
          </p>
        </article>
        <div className="highlight-grid">
          {portfolioHighlights.map((item) => (
            <article className="highlight-tile reveal" key={item.title}>
              <h3>{item.title}</h3>
              <p>{item.body}</p>
            </article>
          ))}
        </div>
      </section>

      <section className="flow-layout">
        <div className="flow-intro">
          <div className="eyebrow">How it works in human language</div>
          <h2>The network behaves less like a directory and more like a venue.</h2>
          <p>
            You can see who is playing, what kind of access they offer, and
            what extra doors friendship might open. But there is still a person
            or policy deciding whether you get backstage.
          </p>
        </div>
        <ol className="flow-list">
          {plainEnglishSteps.map((step) => (
            <li className="flow-step" key={step.title}>
              <h3>{step.title}</h3>
              <p>{step.body}</p>
            </li>
          ))}
        </ol>
      </section>

      <section className="story-grid story-grid--triad">
        <article className="fact-sheet">
          <div className="eyebrow">Consent modes</div>
          <ul className="bullet-list">
            {consentModes.map((mode) => (
              <li key={mode}>{mode}</li>
            ))}
          </ul>
        </article>
        <article className="fact-sheet fact-sheet--ink">
          <div className="eyebrow">Runtime pillars</div>
          <ul className="bullet-list">
            {runtimePillars.map((pillar) => (
              <li key={pillar}>{pillar}</li>
            ))}
          </ul>
        </article>
        <article className="fact-sheet">
          <div className="eyebrow">Product surface</div>
          <ul className="bullet-list">
            {productSurfaces.map((surface) => (
              <li key={surface}>{surface}</li>
            ))}
          </ul>
        </article>
      </section>

      <section className="closing-panel">
        <div className="eyebrow">Next stop</div>
        <h2>
          If the portfolio version makes sense, the concept page shows the full
          protocol shape.
        </h2>
        <p>
          That includes the object model, proposal lifecycle, consent modes,
          relay behavior, and the MVP boundary.
        </p>
        <div className="hero-actions">
          <Link className="pill-link" to="/concept">
            Open concept page
          </Link>
        </div>
      </section>
    </SiteShell>
  )
}
