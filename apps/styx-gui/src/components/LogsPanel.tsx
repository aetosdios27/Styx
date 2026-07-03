import { useAppState } from "../state/store";

export function LogsPanel() {
  const state = useAppState();

  return (
    <section className="panel full-panel">
      <div className="panel-heading">
        <h2>Logs</h2>
        <span>{state.snapshot.logs.length}</span>
      </div>
      <div className="log-list">
        {state.snapshot.logs.length === 0 ? (
          <p className="muted">No log entries.</p>
        ) : (
          state.snapshot.logs.map((line, index) => (
            <div className={`log-line ${line.level}`} key={`${line.message}-${index}`}>
              <span>{line.level}</span>
              <p>{line.message}</p>
            </div>
          ))
        )}
      </div>
    </section>
  );
}
