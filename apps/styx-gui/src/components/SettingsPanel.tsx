export function SettingsPanel() {
  return (
    <section className="panel full-panel settings-grid">
      <div className="panel-heading">
        <h2>Settings</h2>
      </div>
      <label>
        Download directory
        <input readOnly value="Not configured" />
      </label>
      <label>
        Listen port
        <input readOnly value="6881" />
      </label>
      <label>
        Download limit
        <input readOnly value="Unlimited" />
      </label>
      <label className="toggle-row">
        <input checked readOnly type="checkbox" />
        Privacy-first identity rotation
      </label>
    </section>
  );
}
