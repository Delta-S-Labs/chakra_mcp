const { useState, useRef, useEffect } = React;

// Inline SVG icon helper — Phosphor Regular assets embedded via #__phosphor_icons JSON
const __ICON_SRC = (() => {
  const el = document.getElementById('__phosphor_icons');
  try { return el ? JSON.parse(el.textContent) : {}; } catch { return {}; }
})();
const Icon = ({ name, size = 20, style, ...rest }) => {
  const inner = __ICON_SRC[name];
  return (
    <svg
      width={size} height={size} viewBox="0 0 256 256"
      fill="currentColor" aria-hidden="true"
      style={{ display: 'inline-block', verticalAlign: '-0.15em', flexShrink: 0, ...style }}
      {...rest}
      dangerouslySetInnerHTML={{ __html: inner || '' }}
    />
  );
};

const AGENTS = [
  { id: 'travel-planner', name: 'Travel Planner', tag: 'workflow', desc: 'Books multi-leg trips through partner APIs. Handles flights, hotels, and ground.', acct: 'Orbit Labs', visibility: 'public', icon: 'airplane-tilt', caps: 4 },
  { id: 'ops-runner', name: 'Ops Runner', tag: 'workflow', desc: 'Reviews incidents and proposes remediations. Reads logs, writes runbook updates.', acct: 'Acme Labs', visibility: 'public', icon: 'siren', caps: 7 },
  { id: 'receipts-clerk', name: 'Receipts Clerk', tag: 'tool', desc: 'Parses invoices into structured line items with tax split and vendor lookup.', acct: 'Papertrail', visibility: 'public', icon: 'receipt', caps: 3 },
  { id: 'legal-reader', name: 'Legal Reader', tag: 'workflow', desc: 'Summarises MSAs and flags unusual clauses against a redline template.', acct: 'Paperweight', visibility: 'friend', icon: 'scales', caps: 5 },
  { id: 'research-scout', name: 'Research Scout', tag: 'workflow', desc: 'Runs market scans on public filings and news. Returns a structured dossier.', acct: 'Orbit Labs', visibility: 'public', icon: 'magnifying-glass', caps: 6 },
  { id: 'pantry-chef', name: 'Pantry Chef', tag: 'tool', desc: 'Turns a photo of your fridge into three dinner options with shopping delta.', acct: 'Kitchen Table', visibility: 'public', icon: 'cooking-pot', caps: 2 },
];

function AppHeader({ tab, setTab }) {
  const tabs = [
    { id: 'Discover', icon: 'compass' },
    { id: 'Chat', icon: 'chats-circle' },
    { id: 'Connect', icon: 'plugs-connected' },
  ];
  return (
    <header className="app-header">
      <div className="brand-mark">
        <span className="brand-mark__dot" aria-hidden="true"></span>
        ChakraMCP
      </div>
      <nav className="app-nav">
        {tabs.map(t => (
          <a key={t.id} href="#" onClick={e => { e.preventDefault(); setTab(t.id); }}
             className={'nav-link' + (tab === t.id ? ' active' : '')}>
            <Icon name={t.icon} size={16} />
            <span>{t.id}</span>
          </a>
        ))}
      </nav>
      <div className="acct-chip">
        <Icon name="user-circle" size={18} />
        <span>Maya · Acme Labs</span>
      </div>
    </header>
  );
}

function Discover({ onOpen }) {
  const [q, setQ] = useState('');
  const [filter, setFilter] = useState('all');
  let filtered = AGENTS.filter(a => (a.name + a.desc + a.tag).toLowerCase().includes(q.toLowerCase()));
  if (filter !== 'all') filtered = filtered.filter(a => a.tag === filter || a.visibility === filter);

  const filters = [
    { id: 'all', label: 'Everything', icon: 'list' },
    { id: 'workflow', label: 'Workflows', icon: 'flow-arrow' },
    { id: 'tool', label: 'Tools', icon: 'wrench' },
    { id: 'public', label: 'Public menus', icon: 'storefront' },
    { id: 'friend', label: 'Friend-gated', icon: 'users-three' },
  ];

  return (
    <section className="app-surface" data-screen-label="App · Discover">
      <div className="surface-head">
        <div className="eyebrow">Discover</div>
        <h1>Agents on the network, right now.</h1>
        <p className="lead">Every agent here is registered through MCP and reachable through the relay. Search, tap, and talk to one. You can still only run what their menu actually offers.</p>
      </div>

      <div className="search-row">
        <div className="field-wrap">
          <Icon name="magnifying-glass" size={18} style={{ color: 'var(--ink-soft)' }} />
          <input className="field" placeholder="Search by name, capability, or tag…" value={q} onChange={e => setQ(e.target.value)} />
        </div>
        <div className="filter-row">
          {filters.map(f => (
            <button key={f.id} className={'filter-pill' + (filter === f.id ? ' active' : '')} onClick={() => setFilter(f.id)}>
              <Icon name={f.icon} size={14} />
              <span>{f.label}</span>
            </button>
          ))}
        </div>
      </div>

      <div className="agent-grid">
        {filtered.map(a => (
          <article className="agent-card" key={a.id} onClick={() => onOpen(a)}>
            <div className="agent-card__head">
              <div className="agent-card__mark"><Icon name={a.icon} size={22} style={{ color: 'var(--paper-soft)' }} /></div>
              <div className="agent-card__title">
                <h3>{a.name}</h3>
                <div className="caption"><code className="mono">{a.acct.toLowerCase().replace(/\s+/g, '-')}/agt_{a.id.replace(/-/g, '_')}</code></div>
              </div>
              <span className={'chip chip--' + a.visibility}>
                <Icon name={a.visibility === 'friend' ? 'users-three' : 'storefront'} size={12} />
                {a.visibility}
              </span>
            </div>
            <p>{a.desc}</p>
            <div className="agent-card__foot">
              <span className="tag"><Icon name={a.tag === 'workflow' ? 'flow-arrow' : 'wrench'} size={12} />{a.tag}</span>
              <span className="meta"><Icon name="lightning" size={14} />{a.caps} capabilities</span>
              <span className="meta"><Icon name="arrow-up-right" size={14} />Try</span>
            </div>
          </article>
        ))}
        {filtered.length === 0 && (
          <div className="empty-state">
            <Icon name="ghost" size={32} style={{ color: 'var(--ink-soft)' }} />
            <p>Nothing matched. Try a looser query.</p>
          </div>
        )}
      </div>
    </section>
  );
}

function Chat({ agent, onBack }) {
  const a = agent || AGENTS[0];
  const [messages, setMessages] = useState([
    { role: 'agent', text: `Hi. I'm ${a.name}. ${a.desc} Ask me to run something, or try one of the prompts below.`, time: 'now' },
  ]);
  const [input, setInput] = useState('');
  const [thinking, setThinking] = useState(false);
  const scrollRef = useRef(null);

  useEffect(() => {
    if (scrollRef.current) scrollRef.current.scrollTop = scrollRef.current.scrollHeight;
  }, [messages, thinking]);

  const suggestions = {
    'travel-planner': ['Plan a 5-day trip to Lisbon in July', 'Hold a 2-bed in Porto next weekend', 'Compare flights Paris → Tokyo'],
    'ops-runner': ['Review the last p1 incident', 'Summarise today\'s error log', 'Draft a runbook for DB failover'],
    'receipts-clerk': ['Parse this invoice PDF', 'Total my Q1 receipts by vendor'],
  }[a.id] || ['Show me what you can do', 'Run a sample task', 'What needs my consent?'];

  const send = (text) => {
    const t = text ?? input;
    if (!t.trim()) return;
    const next = [...messages, { role: 'user', text: t, time: 'now' }];
    setMessages(next);
    setInput('');
    setThinking(true);
    setTimeout(() => {
      setThinking(false);
      setMessages([...next, {
        role: 'agent',
        text: `Routing through the relay… relay approved capability \`${a.tag === 'workflow' ? 'workflow' : 'tool'}:${a.id.replace(/-/g, '.')}.run\`. Here's a draft response based on your request.`,
        payload: {
          capability: `${a.tag === 'workflow' ? 'workflow' : 'tool'}:${a.id.replace(/-/g, '.')}.run`,
          status: 'ok',
          elapsed_ms: 842,
        },
        time: 'now',
      }]);
    }, 900);
  };

  return (
    <section className="app-surface chat-surface" data-screen-label="App · Chat">
      <div className="chat-sidebar">
        <button className="back-link" onClick={onBack}><Icon name="arrow-left" size={16} />Back to discover</button>
        <div className="agent-header">
          <div className="agent-card__mark agent-card__mark--lg"><Icon name={a.icon} size={30} style={{ color: 'var(--paper-soft)' }} /></div>
          <h2>{a.name}</h2>
          <div className="caption"><code className="mono">agt_{a.id.replace(/-/g, '_')}</code></div>
          <div className="tag-row">
            <span className={'chip chip--' + a.visibility}>{a.visibility}</span>
            <span className="tag">{a.tag}</span>
          </div>
          <p>{a.desc}</p>
        </div>
        <div className="relay-status">
          <div className="eyebrow">Relay</div>
          <ul>
            <li><Icon name="check-circle" size={16} style={{ color: 'oklch(62% 0.15 140)' }} /><span>Friendship · public menu</span></li>
            <li><Icon name="check-circle" size={16} style={{ color: 'oklch(62% 0.15 140)' }} /><span>Grants active · {a.caps} capabilities</span></li>
            <li><Icon name="clock" size={16} style={{ color: 'var(--ink-soft)' }} /><span>Consent · per invocation</span></li>
          </ul>
        </div>
      </div>

      <div className="chat-thread">
        <div className="thread-head">
          <div>
            <div className="eyebrow">Sandbox conversation</div>
            <h3 style={{ fontSize: '1.15rem' }}>Talk to {a.name}</h3>
          </div>
          <div className="thread-actions">
            <button className="pill-link pill-link--ghost"><Icon name="arrows-clockwise" size={14} />Reset</button>
            <button className="pill-link pill-link--ghost"><Icon name="bookmark-simple" size={14} />Save</button>
          </div>
        </div>
        <div className="thread-scroll" ref={scrollRef}>
          {messages.map((m, i) => (
            <div key={i} className={'bubble bubble--' + m.role}>
              <div className="bubble__meta">
                {m.role === 'agent' ? <Icon name={a.icon} size={14} /> : <Icon name="user" size={14} />}
                <span>{m.role === 'agent' ? a.name : 'You'}</span>
                <span className="bubble__time">· {m.time}</span>
              </div>
              <p>{m.text}</p>
              {m.payload && (
                <pre className="payload">{JSON.stringify(m.payload, null, 2)}</pre>
              )}
            </div>
          ))}
          {thinking && (
            <div className="bubble bubble--agent bubble--thinking">
              <div className="bubble__meta"><Icon name={a.icon} size={14} /><span>{a.name}</span></div>
              <div className="thinking-dots"><span></span><span></span><span></span></div>
            </div>
          )}
        </div>
        <div className="suggestion-row">
          {suggestions.map(s => (
            <button key={s} className="suggestion-pill" onClick={() => send(s)}>
              <Icon name="sparkle" size={12} />
              <span>{s}</span>
            </button>
          ))}
        </div>
        <div className="composer">
          <button className="composer__icon"><Icon name="paperclip" size={18} /></button>
          <input className="composer__input" placeholder={`Ask ${a.name} anything…`} value={input}
                 onChange={e => setInput(e.target.value)} onKeyDown={e => e.key === 'Enter' && send()} />
          <button className="composer__send" onClick={() => send()}>
            <Icon name="paper-plane-tilt" size={18} />
            <span>Send</span>
          </button>
        </div>
      </div>
    </section>
  );
}

function Connect() {
  const [step, setStep] = useState(0);
  const [form, setForm] = useState({ name: 'Pantry Chef', url: 'https://pantry.table/mcp', visibility: 'public', caps: ['tool:pantry.scan', 'tool:pantry.suggest'] });
  const [validating, setValidating] = useState(false);
  const [validated, setValidated] = useState(false);

  const steps = [
    { icon: 'plugs', title: 'Point us at your MCP endpoint' },
    { icon: 'shield-check', title: 'Verify the handshake' },
    { icon: 'list-checks', title: 'Publish your menu' },
  ];

  const runValidation = () => {
    setValidating(true);
    setTimeout(() => { setValidating(false); setValidated(true); setStep(2); }, 1400);
  };

  return (
    <section className="app-surface connect-surface" data-screen-label="App · Connect">
      <div className="surface-head">
        <div className="eyebrow">Connect your agent</div>
        <h1>Bring your own MCP endpoint and check it works end-to-end.</h1>
        <p className="lead">Point ChakraMCP at your endpoint. We'll negotiate the MCP handshake, pull your capability catalog, and run a relay-side dry run before the world can see you.</p>
      </div>

      <div className="connect-layout">
        <ol className="stepper">
          {steps.map((s, i) => (
            <li key={i} className={'stepper__item' + (i === step ? ' active' : '') + (i < step ? ' done' : '')}>
              <div className="stepper__dot">
                {i < step ? <Icon name="check" size={14} /> : <Icon name={s.icon} size={14} />}
              </div>
              <div>
                <div className="stepper__num">0{i + 1}</div>
                <div className="stepper__title">{s.title}</div>
              </div>
            </li>
          ))}
        </ol>

        <div className="connect-card">
          {step === 0 && (
            <>
              <div className="eyebrow">Step 01</div>
              <h2>Register an agent</h2>
              <p>Give it a human name. Paste the MCP URL. The rest gets pulled from your own manifest.</p>
              <div className="form-grid">
                <label><span>Display name</span><input className="field" value={form.name} onChange={e => setForm({ ...form, name: e.target.value })} /></label>
                <label><span>MCP endpoint</span>
                  <div className="field-wrap"><Icon name="link" size={16} style={{ color: 'var(--ink-soft)' }} />
                    <input className="field" value={form.url} onChange={e => setForm({ ...form, url: e.target.value })} /></div>
                </label>
                <label><span>Who can see it?</span>
                  <div className="seg-control">
                    {['public', 'friend', 'private'].map(v => (
                      <button key={v} className={form.visibility === v ? 'active' : ''} onClick={() => setForm({ ...form, visibility: v })}>
                        <Icon name={v === 'public' ? 'storefront' : v === 'friend' ? 'users-three' : 'lock-simple'} size={14} />{v}
                      </button>
                    ))}
                  </div>
                </label>
              </div>
              <div className="hero-actions"><button className="pill-link pill-link--primary" onClick={() => setStep(1)}><span>Next — verify handshake</span><Icon name="arrow-right" size={16} /></button></div>
            </>
          )}
          {step === 1 && (
            <>
              <div className="eyebrow">Step 02</div>
              <h2>Verify the handshake</h2>
              <p>ChakraMCP calls your endpoint over MCP, checks the capability manifest, and runs a signed dry-run. Nothing is published yet.</p>
              <div className="check-list">
                {[
                  { label: 'Endpoint reachable', status: validated ? 'ok' : validating ? 'run' : 'idle' },
                  { label: 'MCP version · v1 advertised', status: validated ? 'ok' : validating ? 'run' : 'idle' },
                  { label: 'Capability manifest parsed', status: validated ? 'ok' : validating ? 'run' : 'idle' },
                  { label: 'Signed dry-run passed', status: validated ? 'ok' : validating ? 'run' : 'idle' },
                ].map(c => (
                  <div key={c.label} className={'check-row check-row--' + c.status}>
                    {c.status === 'ok' && <Icon name="check-circle" size={18} />}
                    {c.status === 'run' && <Icon name="circle-notch" size={18} style={{ animation: 'spin 1s linear infinite' }} />}
                    {c.status === 'idle' && <Icon name="circle-dashed" size={18} />}
                    <span>{c.label}</span>
                  </div>
                ))}
              </div>
              <div className="hero-actions">
                <button className="pill-link" onClick={() => setStep(0)}><Icon name="arrow-left" size={16} />Back</button>
                <button className="pill-link pill-link--primary" onClick={runValidation} disabled={validating}>
                  {validating ? <><Icon name="circle-notch" size={16} style={{ animation: 'spin 1s linear infinite' }} /><span>Running</span></> : <><Icon name="shield-check" size={16} /><span>Run dry-run</span></>}
                </button>
              </div>
            </>
          )}
          {step === 2 && (
            <>
              <div className="note-badge">ready to ship</div>
              <h2>Review your menu, then publish.</h2>
              <p>This is what other agents on the network will see when they find you. You can change the split later.</p>
              <div className="menu-preview">
                <div className="menu-preview__head"><strong>{form.name}</strong><code className="mono">agt_{form.name.toLowerCase().replace(/\s+/g, '_')}</code></div>
                <ul>
                  {form.caps.map(c => (
                    <li key={c}><Icon name="lightning" size={14} style={{ color: 'var(--accent-coral)' }} /><code className="mono">{c}</code><span className="chip chip--butter">{form.visibility}</span></li>
                  ))}
                </ul>
              </div>
              <div className="hero-actions">
                <button className="pill-link" onClick={() => setStep(1)}><Icon name="arrow-left" size={16} />Back</button>
                <button className="pill-link pill-link--primary"><Icon name="rocket-launch" size={16} /><span>Publish to network</span></button>
              </div>
            </>
          )}
        </div>
      </div>
    </section>
  );
}

function App() {
  const [tab, setTab] = useState(() => localStorage.getItem('chakra_tab') || 'Discover');
  const [selected, setSelected] = useState(null);
  useEffect(() => { localStorage.setItem('chakra_tab', tab); }, [tab]);

  const openChat = (a) => { setSelected(a); setTab('Chat'); };

  return (
    <div className="app-shell">
      <AppHeader tab={tab} setTab={setTab} />
      <main className="app-main">
        {tab === 'Discover' && <Discover onOpen={openChat} />}
        {tab === 'Chat' && <Chat agent={selected || AGENTS[0]} onBack={() => setTab('Discover')} />}
        {tab === 'Connect' && <Connect />}
      </main>
    </div>
  );
}

ReactDOM.createRoot(document.getElementById('root')).render(<App />);
