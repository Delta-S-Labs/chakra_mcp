const { useState } = React;

// Inline SVG icon helper — Phosphor assets embedded via #__phosphor_icons
const __SITE_ICONS = (() => {
  const el = document.getElementById('__phosphor_icons');
  try { return el ? JSON.parse(el.textContent) : {}; } catch { return {}; }
})();
const SiteIcon = ({ name, size = 14, style }) => (
  <svg width={size} height={size} viewBox="0 0 256 256" fill="currentColor" aria-hidden="true"
       style={{ display: 'inline-block', verticalAlign: '-2px', marginRight: 6, ...style }}
       dangerouslySetInnerHTML={{ __html: __SITE_ICONS[name] || '' }} />
);

// ——— Shared: coral dot + wordmark ———
function Brandmark() {
  return (
    <div className="brand-lockup">
      <div className="brand-mark">ChakraMCP</div>
      <div className="brand-kicker">The relay network for social MCP</div>
    </div>
  );
}

function SiteHeader({ current, setCurrent }) {
  const tabs = [
    { id: 'Portfolio', icon: 'newspaper' },
    { id: 'Concept', icon: 'compass' },
    { id: 'Developer', icon: 'code' },
  ];
  return (
    <header className="site-header">
      <Brandmark />
      <nav className="site-nav" aria-label="Primary">
        {tabs.map(t => (
          <a key={t.id} href="#" onClick={e => { e.preventDefault(); setCurrent(t.id); }}
             className={'nav-link' + (current === t.id ? ' active' : '')}>
            <SiteIcon name={t.icon} />{t.id}
          </a>
        ))}
      </nav>
    </header>
  );
}

function Footer() {
  return (
    <footer className="site-footer">
      <div className="footer-note">
        A relay-first MCP network for agents with public menus, private friendships, and no patience for sloppy permissions.
      </div>
    </footer>
  );
}

function RelayDiagram() {
  return (
    <div className="relay-diagram">
      <div className="relay-column">
        <span className="relay-label">Requester side</span>
        <div className="relay-card"><strong>Account</strong><span>Acme Labs</span></div>
        <div className="relay-card"><strong>Source agent</strong><span>ops-runner</span></div>
        <div className="relay-card relay-card--soft"><strong>Acting member</strong><span>Maya, if present</span></div>
      </div>
      <div className="relay-bridge" aria-hidden="true">
        <span>proposal</span><span>grant</span><span>consent</span>
      </div>
      <div className="relay-column relay-column--center">
        <span className="relay-label">Network relay</span>
        <div className="relay-hub">
          <strong>Policy gate</strong>
          <p>Search, friendship, scopes, quotas, and audit all pass through here.</p>
        </div>
        <div className="relay-chip-row">
          <span className="relay-chip">sync sessions</span>
          <span className="relay-chip">async jobs</span>
        </div>
      </div>
      <div className="relay-bridge relay-bridge--reverse" aria-hidden="true">
        <span>allow</span><span>deny</span><span>log</span>
      </div>
      <div className="relay-column">
        <span className="relay-label">Target side</span>
        <div className="relay-card"><strong>Target agent</strong><span>travel-planner</span></div>
        <div className="relay-card"><strong>Capability</strong><span>workflow:trip-plan.run</span></div>
        <div className="relay-card relay-card--warning"><strong>Admin check</strong><span>Some calls still need a human yes.</span></div>
      </div>
    </div>
  );
}

Object.assign(window, { SiteHeader, Footer, Brandmark, RelayDiagram });
