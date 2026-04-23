import { RelayDiagram } from '../components/RelayDiagram'
import { SiteShell } from '../components/SiteShell'
import {
  conceptPrimer,
  conceptSections,
  consentModes,
  coreObjects,
  mvpExcludes,
  mvpIncludes,
  plainEnglishSteps,
  relayReasons,
  surfaceNarrative,
} from '../content/concept'

export function ConceptPage() {
  return (
    <SiteShell kicker="Full concept page for the MCP friendship protocol">
      <section className="hero-block hero-block--concept">
        <div className="hero-copy reveal">
          <div className="eyebrow">Concept page</div>
          <h1>A trust network for agents that can actually say no.</h1>
          <p className="lead">
            In plain English: agents can advertise what they do, show what
            friendship unlocks, and still require approval when something
            sensitive is about to run.
          </p>
        </div>
        <div className="hero-board hero-board--concept reveal">
          <RelayDiagram variant="full" />
        </div>
      </section>

      <section className="concept-at-a-glance" id="overview">
        <div className="section-head section-head--compact">
          <div className="eyebrow">Overview</div>
          <h2>The whole idea in three moves.</h2>
          <p>
            This page gets more detailed as you go. If you only need the shape,
            start here.
          </p>
        </div>
        <div className="glance-grid">
          {conceptPrimer.map((item) => (
            <article className="glance-card" key={item.title}>
              <h3>{item.title}</h3>
              <p>{item.body}</p>
            </article>
          ))}
        </div>
      </section>

      <section className="concept-rail" aria-label="Concept sections">
        <span className="concept-rail__label">Concept deck</span>
        <nav className="concept-rail__nav">
          {conceptSections.map(([id, label], index) => (
            <a className="concept-rail__link" href={`#${id}`} key={id}>
              <span>{String(index + 1).padStart(2, '0')}</span>
              {label}
            </a>
          ))}
        </nav>
      </section>

      <section className="concept-stage concept-stage--objects" id="objects">
        <div className="chapter-marker">01</div>
        <div className="concept-stage__body">
          <div className="section-head">
            <div className="eyebrow">Objects</div>
            <h2>Who exists in this network?</h2>
            <p>
              The model separates relationship, execution, and human context so
              permission logic does not turn into mush.
            </p>
          </div>
          <dl className="object-grid object-grid--staggered">
            {coreObjects.map(([name, description]) => (
              <div className="object-item" key={name}>
                <dt>{name}</dt>
                <dd>{description}</dd>
              </div>
            ))}
          </dl>
        </div>
      </section>

      <section className="concept-stage concept-stage--flow" id="flow">
        <div className="chapter-marker">02</div>
        <div className="concept-stage__body">
          <div className="section-head">
            <div className="eyebrow">Flow</div>
            <h2>How one side gets access to the other without guesswork.</h2>
            <p>
              The network handles trust as a negotiated process, not as a binary
              “friend or stranger” toggle.
            </p>
          </div>
          <ol className="timeline-list">
            {plainEnglishSteps.map((step, index) => (
              <li className="timeline-step" key={step.title}>
                <span className="timeline-step__index">
                  {String(index + 1).padStart(2, '0')}
                </span>
                <div className="timeline-step__content">
                  <h3>{step.title}</h3>
                  <p>{step.body}</p>
                </div>
              </li>
            ))}
          </ol>
        </div>
      </section>

      <section className="concept-stage concept-stage--consent" id="consent">
        <div className="chapter-marker">03</div>
        <div className="concept-stage__body">
          <div className="section-head">
            <div className="eyebrow">Consent</div>
            <h2>Some workflows should stay annoying on purpose.</h2>
            <p>
              Not every capability deserves to become a silent background
              permission. Some need a deliberate yes.
            </p>
          </div>
          <div className="consent-band">
            {consentModes.map((mode) => (
              <article className="consent-mode" key={mode}>
                <p>{mode}</p>
              </article>
            ))}
          </div>
          <p className="stage-note">
            Sensitive tools can also be marked as always requiring owner or
            admin approval, even when friendship already exists.
          </p>
        </div>
      </section>

      <section className="concept-stage concept-stage--runtime" id="runtime">
        <div className="chapter-marker">04</div>
        <div className="concept-stage__body">
          <div className="section-head">
            <div className="eyebrow">Runtime</div>
            <h2>The relay is where the network stops being vibes and becomes infrastructure.</h2>
            <p>
              Direct agent-to-agent access sounds elegant until you need routing,
              revocation, audit history, or a way to stop bad calls before they
              land.
            </p>
          </div>
          <div className="runtime-layout">
            <div className="runtime-note">
              <p>
                The relay is the shared checkpoint. It verifies identity,
                friendship state, grants, quotas, and consent. The target agent
                can still refuse the call afterward.
              </p>
            </div>
            <div className="reason-grid reason-grid--runtime">
              {relayReasons.map((reason) => (
                <article className="reason-card" key={reason.title}>
                  <h3>{reason.title}</h3>
                  <p>{reason.body}</p>
                </article>
              ))}
            </div>
          </div>
        </div>
      </section>

      <section className="concept-stage concept-stage--surface" id="surface">
        <div className="chapter-marker">05</div>
        <div className="concept-stage__body">
          <div className="section-head">
            <div className="eyebrow">Product</div>
            <h2>What people actually see and use.</h2>
            <p>
              A protocol alone is not enough. The network also needs a product
              surface that makes all this trust choreography understandable.
            </p>
          </div>
          <ol className="surface-stack">
            {surfaceNarrative.map((item, index) => (
              <li className="surface-stack__item" key={item}>
                <span className="surface-stack__index">
                  {String(index + 1).padStart(2, '0')}
                </span>
                <p>{item}</p>
              </li>
            ))}
          </ol>
        </div>
      </section>

      <section className="concept-stage concept-stage--mvp" id="mvp">
        <div className="chapter-marker">06</div>
        <div className="concept-stage__body">
          <div className="section-head">
            <div className="eyebrow">MVP</div>
            <h2>What ships first, and what can wait outside the room.</h2>
            <p>
              The first version should be coherent, not maximal. This cut keeps
              the network useful without pretending it has already solved every
              trust problem on Earth.
            </p>
          </div>
          <div className="boundary-board">
            <div className="boundary-column">
              <h3>Include</h3>
              <ul className="bullet-list">
                {mvpIncludes.map((item) => (
                  <li key={item}>{item}</li>
                ))}
              </ul>
            </div>
            <div className="boundary-column boundary-column--muted">
              <h3>Exclude</h3>
              <ul className="bullet-list">
                {mvpExcludes.map((item) => (
                  <li key={item}>{item}</li>
                ))}
              </ul>
            </div>
          </div>
        </div>
      </section>
    </SiteShell>
  )
}
