import { formatBytes, formatPercent } from "../lib/format";
import { useAppState } from "../state/store";
import { PieceHeatmap } from "./PieceHeatmap";

export function TorrentDetail() {
  const state = useAppState();
  const torrent = state.snapshot.torrents.find((row) => row.info_hash === state.selectedInfoHash);

  if (!torrent) {
    return (
      <aside className="panel detail-panel">
        <div className="panel-heading">
          <h2>Detail</h2>
        </div>
        <p className="muted">Select a torrent to inspect metadata, peers, and pieces.</p>
      </aside>
    );
  }

  return (
    <aside className="panel detail-panel">
      <div className="panel-heading">
        <h2>{torrent.name}</h2>
      </div>
      <dl className="kv">
        <div>
          <dt>Info hash</dt>
          <dd className="hash">{torrent.info_hash}</dd>
        </div>
        <div>
          <dt>Status</dt>
          <dd>{torrent.status}</dd>
        </div>
        <div>
          <dt>Size</dt>
          <dd>{formatBytes(torrent.size_bytes)}</dd>
        </div>
        <div>
          <dt>Progress</dt>
          <dd>{formatPercent(torrent.progress)}</dd>
        </div>
      </dl>
      <PieceHeatmap progress={torrent.progress} />
    </aside>
  );
}
