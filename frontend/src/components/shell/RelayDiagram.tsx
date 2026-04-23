export default function RelayDiagram() {
  return (
    <div className="relay-diagram">
      <div className="relay-column">
        <span className="relay-label">Requester side</span>
        <div className="relay-card">
          <strong>Account</strong>
          <span>Acme Labs</span>
        </div>
        <div className="relay-card">
          <strong>Source agent</strong>
          <span>ops-runner</span>
        </div>
        <div className="relay-card relay-card--soft">
          <strong>Acting member</strong>
          <span>Maya, if present</span>
        </div>
      </div>
      <div className="relay-bridge" aria-hidden="true">
        <span>proposal</span>
        <span>grant</span>
        <span>consent</span>
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
        <span>allow</span>
        <span>deny</span>
        <span>log</span>
      </div>
      <div className="relay-column">
        <span className="relay-label">Target side</span>
        <div className="relay-card">
          <strong>Target agent</strong>
          <span>travel-planner</span>
        </div>
        <div className="relay-card">
          <strong>Capability</strong>
          <span>workflow:trip-plan.run</span>
        </div>
        <div className="relay-card relay-card--warning">
          <strong>Admin check</strong>
          <span>Some calls still need a human yes.</span>
        </div>
      </div>
    </div>
  );
}
