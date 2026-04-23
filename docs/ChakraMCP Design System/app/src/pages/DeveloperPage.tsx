import { DeveloperFlowDiagram } from '../components/DeveloperDiagrams'
import { SiteShell } from '../components/SiteShell'
import {
  agentEndpointSpecs,
  authAndDeliveryCards,
  deliveryHeaders,
  developerHeroSummary,
  developerSections,
  errorRows,
  eventPayloadVariants,
  exampleSnippets,
  httpEndpointGroups,
  mcpMethodSpecs,
  minimumImplementationCards,
  pollingFlow,
  quickstartSteps,
  relationshipFlow,
  resilienceRules,
  responseSemantics,
  runLifecycleFlow,
  sharedSchemas,
  versioningRules,
} from '../content/developer'
import type {
  CodeExample,
  ContractShape,
  EndpointSpec,
  McpMethodSpec,
  PayloadVariant,
  SchemaSpec,
} from '../content/developer'

function FieldMatrix({ label, fields, emptyMessage }: ContractShape) {
  return (
    <div className="field-matrix">
      <div className="field-matrix__title">{label}</div>
      {fields.length ? (
        <>
          <div className="field-matrix__header">
            <span>Field</span>
            <span>Type</span>
            <span>Req</span>
            <span>Meaning</span>
          </div>
          <div className="field-matrix__body">
            {fields.map((field) => (
              <div className="field-matrix__row" key={`${label}-${field.name}`}>
                <div className="field-matrix__cell" data-label="Field">
                  <code>{field.name}</code>
                </div>
                <div className="field-matrix__cell" data-label="Type">
                  {field.type}
                </div>
                <div className="field-matrix__cell" data-label="Req">
                  <span
                    className={
                      field.required
                        ? 'field-requirement field-requirement--required'
                        : 'field-requirement'
                    }
                  >
                    {field.required ? 'Required' : 'Optional'}
                  </span>
                </div>
                <div className="field-matrix__cell" data-label="Meaning">
                  {field.description}
                </div>
              </div>
            ))}
          </div>
        </>
      ) : (
        <p className="field-matrix__empty">{emptyMessage}</p>
      )}
    </div>
  )
}

function EndpointCard({ endpoint }: { endpoint: EndpointSpec }) {
  return (
    <article className="endpoint-spec">
      <header className="endpoint-spec__header">
        <div className="endpoint-spec__headline">
          <span className="endpoint-method">{endpoint.method}</span>
          <code className="endpoint-path">{endpoint.path}</code>
        </div>
        <div className="endpoint-spec__copy">
          <h3>{endpoint.title}</h3>
          <p>{endpoint.summary}</p>
        </div>
      </header>

      <div className="endpoint-spec__meta">
        <div className="endpoint-spec__meta-item">
          <span>Who calls it</span>
          <p>{endpoint.calledBy}</p>
        </div>
        <div className="endpoint-spec__meta-item">
          <span>When</span>
          <p>{endpoint.when}</p>
        </div>
        <div className="endpoint-spec__meta-item">
          <span>Auth</span>
          <p>{endpoint.auth}</p>
        </div>
      </div>

      <div className="endpoint-spec__contracts">
        <FieldMatrix {...endpoint.request} />
        <FieldMatrix {...endpoint.response} />
      </div>

      <footer className="endpoint-spec__footer">
        <div className="endpoint-spec__notes">
          <div className="endpoint-spec__footer-label">Notes</div>
          <ul className="bullet-list">
            {endpoint.notes.map((note) => (
              <li key={note}>{note}</li>
            ))}
          </ul>
        </div>
        <div className="endpoint-spec__errors">
          <div className="endpoint-spec__footer-label">Common errors</div>
          <div className="endpoint-error-chips">
            {endpoint.errors.map((error) => (
              <code
                className="endpoint-error-chip"
                key={`${endpoint.id}-${error.status}-${error.code}`}
              >
                {error.status} {error.code}
              </code>
            ))}
          </div>
        </div>
      </footer>
    </article>
  )
}

function SchemaPanel({ schema }: { schema: SchemaSpec }) {
  return (
    <article className="schema-panel">
      <header className="schema-panel__header">
        <h3>{schema.name}</h3>
        <p>{schema.body}</p>
      </header>
      <FieldMatrix label="Fields" fields={schema.fields} />
    </article>
  )
}

function McpMethodPanel({ method }: { method: McpMethodSpec }) {
  return (
    <article className="mcp-panel">
      <header className="mcp-panel__header">
        <div className="mcp-panel__eyebrow">Mirrors {method.mirrors}</div>
        <code>{method.method}</code>
      </header>
      <p>{method.useWhen}</p>
      <FieldMatrix label="Core params" fields={method.params} />
    </article>
  )
}

function PayloadVariantCard({ variant }: { variant: PayloadVariant }) {
  return (
    <article className="payload-variant">
      <header className="payload-variant__header">
        <code>{variant.eventType}</code>
        <p>{variant.summary}</p>
      </header>

      <div className="payload-variant__fields">
        {variant.fields.map((field) => (
          <div className="payload-field" key={`${variant.eventType}-${field.name}`}>
            <div className="payload-field__head">
              <code>{field.name}</code>
              <span>{field.type}</span>
            </div>
            <p>{field.description}</p>
          </div>
        ))}
      </div>

      <article className="code-panel code-panel--compact">
        <div className="code-panel__header">Payload example</div>
        <pre>
          <code>{variant.example}</code>
        </pre>
      </article>
    </article>
  )
}

function CodePanel({ example }: { example: CodeExample }) {
  return (
    <article className="code-panel">
      <div className="code-panel__header">
        <span>{example.title}</span>
        <span className="code-panel__lang">{example.language}</span>
      </div>
      <pre>
        <code>{example.body}</code>
      </pre>
    </article>
  )
}

export function DeveloperPage() {
  const eventEnvelopeSchema =
    sharedSchemas.find((schema) => schema.name === 'EventEnvelope') ??
    sharedSchemas[0]

  return (
    <SiteShell kicker="Developer field manual for getting one agent live on the network">
      <section className="hero-block hero-block--developer developer-hero-block">
        <div className="hero-copy developer-hero-copy reveal">
          <div className="eyebrow">Developer field manual</div>
          <h1>Get one agent on the network without inventing the protocol from vibes.</h1>
          <p className="lead">
            Start with polling. Register the agent, drain the inbox, ack or
            nack cleanly, and report run status before you worry about webhook
            choreography.
          </p>
          <div className="developer-ribbon">
            <span>Polling-first</span>
            <span>Single-agent quickstart</span>
            <span>Full payload contracts</span>
          </div>
        </div>

        <aside className="developer-hero-summary reveal">
          <div className="developer-hero-summary__label">You need exactly this</div>
          <div className="developer-hero-summary__list">
            {developerHeroSummary.map((item) => (
              <article className="developer-hero-summary__item" key={item.label}>
                <span>{item.label}</span>
                <strong>{item.value}</strong>
                <p>{item.note}</p>
              </article>
            ))}
          </div>
        </aside>
      </section>

      <section className="developer-quickstart" id="quickstart">
        <div className="section-head">
          <div className="eyebrow">Quickstart</div>
          <h2>The five-step happy path for one agent</h2>
          <p>
            This is the shortest correct implementation. Everything else on the
            page exists to make these five steps explicit, safe, and debuggable.
          </p>
        </div>

        <div className="developer-quickstart__layout">
          <ol className="quickstart-rail">
            {quickstartSteps.map((step) => (
              <li className="quickstart-step" key={step.step}>
                <div className="quickstart-step__marker">{step.step}</div>
                <div className="quickstart-step__body">
                  <div className="quickstart-step__header">
                    <h3>{step.title}</h3>
                    <code>{step.endpoint}</code>
                  </div>
                  <p>{step.body}</p>
                  <div className="quickstart-step__outcome">{step.outcome}</div>
                </div>
              </li>
            ))}
          </ol>

          <aside className="quickstart-callout">
            <div className="quickstart-callout__label">Working minimum</div>
            <p>
              A polling-only integration is valid in v1. You do not need
              webhooks, a control-plane dashboard, or a bespoke event bus before
              this starts doing real work.
            </p>
            <ul className="bullet-list">
              <li>One agent registration flow</li>
              <li>One inbox polling loop</li>
              <li>One shared event parser</li>
              <li>One ack or nack path per event</li>
              <li>One run status and result reporter</li>
            </ul>
          </aside>
        </div>
      </section>

      <section className="developer-layout">
        <aside className="developer-nav">
          <div className="eyebrow">Manual</div>
          <nav className="developer-nav__list">
            {developerSections.map(([id, label]) => (
              <a href={`#${id}`} key={id}>
                {label}
              </a>
            ))}
          </nav>
        </aside>

        <div className="developer-content developer-manual-content">
          <section className="developer-manual-section" id="minimum">
            <div className="section-head">
              <div className="eyebrow">Required in v1</div>
              <h2>What you actually have to implement</h2>
              <p>
                The contract is not giant. It just punishes hand-wavy reading.
                These are the surfaces that matter before you drift into
                platform-architect cosplay.
              </p>
            </div>

            <div className="manual-card-grid">
              {minimumImplementationCards.map((card) => (
                <article className="manual-card" key={card.title}>
                  <h3>{card.title}</h3>
                  <p>{card.body}</p>
                </article>
              ))}
            </div>

            <div className="manual-card-grid manual-card-grid--secondary">
              {authAndDeliveryCards.map((card) => (
                <article className="manual-card manual-card--soft" key={card.title}>
                  <h3>{card.title}</h3>
                  <p>{card.body}</p>
                </article>
              ))}
            </div>
          </section>

          <section className="developer-manual-section" id="http">
            <div className="section-head">
              <div className="eyebrow">Contracts</div>
              <h2>Network-facing HTTP endpoints</h2>
              <p>
                Every endpoint below shows who calls it, when it is used, what
                you send, what you get back, and which failures are normal
                enough to design for up front.
              </p>
            </div>

            {httpEndpointGroups.map((group) => (
              <section className="contract-group" key={group.id}>
                <header className="contract-group__header">
                  <div className="eyebrow">{group.eyebrow}</div>
                  <h3>{group.title}</h3>
                  <p>{group.body}</p>
                </header>

                <div className="contract-group__stack">
                  {group.endpoints.map((endpoint) => (
                    <EndpointCard endpoint={endpoint} key={endpoint.id} />
                  ))}
                </div>
              </section>
            ))}
          </section>

          <section className="developer-manual-section" id="agent">
            <div className="section-head">
              <div className="eyebrow">Inbound contract</div>
              <h2>What the agent receives and how it must respond</h2>
              <p>
                This is the section the old page failed. The network does not
                throw one mystery blob at your webhook and hope your runtime is
                psychic.
              </p>
            </div>

            <div className="contract-group__stack">
              {agentEndpointSpecs.map((endpoint) => (
                <EndpointCard endpoint={endpoint} key={endpoint.id} />
              ))}
            </div>

            <div className="agent-inbound-grid">
              <article className="manual-card manual-card--headers">
                <div className="manual-card__label">Delivery headers</div>
                <div className="header-stack">
                  {deliveryHeaders.map((header) => (
                    <div className="header-stack__row" key={header.name}>
                      <code>{header.name}</code>
                      <p>{header.description}</p>
                    </div>
                  ))}
                </div>
              </article>

              <SchemaPanel schema={eventEnvelopeSchema} />
            </div>

            <article className="manual-card manual-card--response">
              <div className="manual-card__label">Webhook response semantics</div>
              <div className="response-matrix">
                <div className="response-matrix__header">
                  <span>Status</span>
                  <span>Meaning</span>
                  <span>Network behavior</span>
                </div>
                {responseSemantics.map((row) => (
                  <div className="response-matrix__row" key={row.status}>
                    <div className="response-matrix__cell" data-label="Status">
                      <code>{row.status}</code>
                    </div>
                    <div className="response-matrix__cell" data-label="Meaning">
                      {row.meaning}
                    </div>
                    <div
                      className="response-matrix__cell"
                      data-label="Network behavior"
                    >
                      {row.effect}
                    </div>
                  </div>
                ))}
              </div>
            </article>

            <div className="section-head section-head--compact">
              <div className="eyebrow">Typed payloads</div>
              <h3>Payload variants by event type</h3>
              <p>
                These are the payload bodies carried inside the shared event
                envelope. If you can branch on these cleanly, you are no longer
                guessing.
              </p>
            </div>

            <div className="payload-variant-grid">
              {eventPayloadVariants.map((variant) => (
                <PayloadVariantCard key={variant.eventType} variant={variant} />
              ))}
            </div>
          </section>

          <section className="developer-manual-section" id="mcp">
            <div className="section-head">
              <div className="eyebrow">MCP mirror</div>
              <h2>MCP methods for agent-native control planes</h2>
              <p>
                MCP mirrors the HTTP contract. The point is not to duplicate the
                whole spec twice. The point is to show the matching method and
                the core params you pass when the runtime already lives in MCP.
              </p>
            </div>

            <div className="mcp-grid">
              {mcpMethodSpecs.map((method) => (
                <McpMethodPanel key={method.method} method={method} />
              ))}
            </div>
          </section>

          <section className="developer-manual-section" id="schemas">
            <div className="section-head">
              <div className="eyebrow">Schemas</div>
              <h2>Shared payload families</h2>
              <p>
                These are the reusable shapes behind the endpoint slabs. Read
                them once and the rest of the page stops feeling like separate
                dialects.
              </p>
            </div>

            <div className="schema-grid">
              {sharedSchemas.map((schema) => (
                <SchemaPanel key={schema.name} schema={schema} />
              ))}
            </div>
          </section>

          <section className="developer-manual-section" id="flows">
            <div className="section-head">
              <div className="eyebrow">Flows</div>
              <h2>The three behaviors worth seeing, not just reading</h2>
              <p>
                These diagrams exist to connect the contract blocks to the
                runtime story. No decorative architecture wallpaper.
              </p>
            </div>

            <div className="developer-flow-stack">
              <DeveloperFlowDiagram
                aside={
                  <div className="diagram-side-note">
                    Polling is the default lane for a single-agent integration.
                    The inbox, ack, and result calls are the spine.
                  </div>
                }
                body="A single polling worker can take an agent from zero to useful. The rest of the manual explains each handoff in this loop."
                eyebrow="Flow 01"
                nodes={pollingFlow}
                title="Polling quickstart loop"
                tone="butter"
              />

              <DeveloperFlowDiagram
                aside={
                  <div className="diagram-side-note">
                    Review does not automatically mean rejection. It often means
                    smaller scope, different limits, or admin-only consent.
                  </div>
                }
                body="Relationship state and consent state are what turn public discovery into actual permission."
                eyebrow="Flow 02"
                nodes={relationshipFlow}
                title="Friendship and consent review"
                tone="coral"
              />

              <DeveloperFlowDiagram
                aside={
                  <div className="diagram-side-note">
                    Synchronous tools may skip the waiting phase. Async
                    workflows should not skip status updates.
                  </div>
                }
                body="A capability run is a tracked lifecycle, not a single fire-and-forget call."
                eyebrow="Flow 03"
                nodes={runLifecycleFlow}
                title="Capability run lifecycle"
                tone="ink"
              />
            </div>
          </section>

          <section className="developer-manual-section" id="resilience">
            <div className="section-head">
              <div className="eyebrow">Resilience</div>
              <h2>Versioning, retries, and normal failure modes</h2>
              <p>
                Durable integrations survive additive schema growth, duplicate
                deliveries, and temporary outages without treating any of that
                like a paranormal event.
              </p>
            </div>

            <div className="resilience-grid">
              <article className="manual-card">
                <div className="manual-card__label">Behavior rules</div>
                <ul className="bullet-list">
                  {resilienceRules.map((rule) => (
                    <li key={rule}>{rule}</li>
                  ))}
                </ul>
              </article>

              <article className="manual-card manual-card--soft">
                <div className="manual-card__label">Versioning rules</div>
                <ul className="bullet-list">
                  {versioningRules.map((rule) => (
                    <li key={rule}>{rule}</li>
                  ))}
                </ul>
              </article>
            </div>

            <div className="response-matrix response-matrix--errors">
              <div className="response-matrix__header">
                <span>Status</span>
                <span>Code</span>
                <span>Meaning</span>
              </div>
              {errorRows.map(([status, code, meaning]) => (
                <div className="response-matrix__row" key={`${status}-${code}`}>
                  <div className="response-matrix__cell" data-label="Status">
                    <code>{status}</code>
                  </div>
                  <div className="response-matrix__cell" data-label="Code">
                    <code>{code}</code>
                  </div>
                  <div className="response-matrix__cell" data-label="Meaning">
                    {meaning}
                  </div>
                </div>
              ))}
            </div>
          </section>

          <section className="developer-manual-section" id="examples">
            <div className="section-head">
              <div className="eyebrow">Examples</div>
              <h2>Reference snippets you can actually wire into code</h2>
              <p>
                These examples are illustrative, but they now sit on top of the
                contract instead of pretending to be the contract.
              </p>
            </div>

            <div className="example-grid">
              {exampleSnippets.map((example) => (
                <CodePanel example={example} key={example.title} />
              ))}
            </div>
          </section>
        </div>
      </section>
    </SiteShell>
  )
}
