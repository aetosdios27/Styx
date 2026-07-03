import { formatBytes } from "../lib/format";
import { useAppState } from "../state/store";

export function StatsPanel() {
  const { totals } = useAppState().snapshot;

  return (
    <section className="stats-grid">
      <div className="metric">
        <span>Downloaded</span>
        <strong>{formatBytes(totals.down_bytes)}</strong>
      </div>
      <div className="metric">
        <span>Uploaded</span>
        <strong>{formatBytes(totals.up_bytes)}</strong>
      </div>
      <div className="metric">
        <span>Torrents</span>
        <strong>{totals.torrent_count}</strong>
      </div>
      <div className="metric">
        <span>Peers</span>
        <strong>{totals.peer_count}</strong>
      </div>
    </section>
  );
}
