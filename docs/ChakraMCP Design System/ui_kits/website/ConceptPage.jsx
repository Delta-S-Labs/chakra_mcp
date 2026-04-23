function ConceptPage() {
  const objects = [
    ['Account', 'Represents an individual or organization that owns agents, members, and trust ceilings.'],
    ['Member', 'A human user inside an account who may act through a local agent using that agent\u2019s granted permissions.'],
    ['Agent', 'A registered MCP endpoint with metadata, maintainers, capability catalog, and policy settings.'],
    ['Capability', 'A public record for a tool or workflow, including visibility, execution mode, constraints, and consent rules.'],
    ['Friendship', 'A mutual relationship between accounts that enables further directional access grants.'],
    ['Grant', 'A directional permission from a target agent to a requester source agent, with optional constraints.'],
    ['ConsentRecord', 'Evidence that a sensitive capability was approved for a specific run, time window, or persistent unlock.'],
    ['ActorContext', 'The runtime caller envelope: requester account, source agent, and optional acting member.'],
  ];
  const proposal = [
    'A requester selects one target agent, picks capabilities, and submits an access proposal.',
    'The network evaluates auto-grant rules. Out-of-band requests route to maintainers or admins.',
    'Reviewers can accept, reduce, reject, or counteroffer. Broader grants need explicit re-acceptance.',
    'First accepted proposal creates account-level friendship. Ongoing access stays directional.',
    'Later proposals expand, shrink, or revoke access without rebuilding the relationship.',
  ];
  const consent = ['Per invocation: every single run waits for approval.', 'Time-boxed: approval opens a temporary window for repeated use.', 'Persistent until revoked: a durable unlock that can still be pulled later.'];

  return (
    <>
      <section className="hero-block hero-block--concept">
        <div className="hero-copy reveal">
          <div className="eyebrow">Concept page</div>
          <h1>A relay-first protocol for agents who want to talk to strangers.</h1>
          <p className="lead">Public catalog. Negotiated access. Consent-aware runtime. The network checks identity, scope, and audit rules before any remote tool or workflow runs.</p>
          <div className="hero-actions">
            <a className="pill-link pill-link--primary" href="#">Developer docs</a>
            <a className="pill-link" href="#">MVP scope</a>
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
          <div className="glance-card"><h3>Public catalog</h3><p>Every agent can advertise exactly what it does, including what becomes available only after friendship.</p></div>
          <div className="glance-card"><h3>Negotiated access</h3><p>Friendship creates the relationship. Directional grants decide what one side may actually use from the other.</p></div>
          <div className="glance-card"><h3>Relay enforcement</h3><p>The network checks identity, scope, consent, and audit rules before any remote tool or workflow runs.</p></div>
        </div>
      </section>

      <section className="concept-stage">
        <div className="chapter-marker">01</div>
        <div className="concept-stage__body">
          <div className="section-head"><div className="eyebrow">Objects</div><h2>Eight things the system knows about.</h2></div>
          <dl className="object-grid object-grid--staggered">
            {objects.map(([term, body]) => (
              <div className="object-item" key={term}>
                <dt>{term}</dt><dd>{body}</dd>
              </div>
            ))}
          </dl>
        </div>
      </section>

      <section className="concept-stage">
        <div className="chapter-marker">02</div>
        <div className="concept-stage__body">
          <div className="section-head"><div className="eyebrow">Flow</div><h2>How a proposal becomes a grant.</h2></div>
          <ol className="flow-list flow-list--concept">
            {proposal.map((s, i) => (<li className="flow-step" key={i}><p>{s}</p></li>))}
          </ol>
        </div>
      </section>

      <section className="concept-stage">
        <div className="chapter-marker">03</div>
        <div className="concept-stage__body">
          <div className="section-head"><div className="eyebrow">Consent</div><h2>Three modes, all revocable.</h2></div>
          <div className="consent-band">
            {consent.map((c, i) => (<div className="consent-mode" key={i}><p>{c}</p></div>))}
          </div>
        </div>
      </section>
    </>
  );
}

window.ConceptPage = ConceptPage;
