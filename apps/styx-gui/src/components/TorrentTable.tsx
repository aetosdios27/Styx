import { formatBytes, formatPercent, formatRate } from "../lib/format";
import { useAppDispatch, useAppState } from "../state/store";

export function TorrentTable() {
  const state = useAppState();
  const dispatch = useAppDispatch();

  return (
    <section className="panel table-panel">
      <div className="panel-heading">
        <h2>Torrents</h2>
        <span>{state.snapshot.torrents.length}</span>
      </div>
      <div className="table-wrap">
        <table>
          <thead>
            <tr>
              <th>Name</th>
              <th>State</th>
              <th>Size</th>
              <th>Progress</th>
              <th>Down</th>
              <th>Up</th>
              <th>Seeds</th>
            </tr>
          </thead>
          <tbody>
            {state.snapshot.torrents.length === 0 ? (
              <tr>
                <td colSpan={7} className="empty-cell">
                  No torrents in this session.
                </td>
              </tr>
            ) : (
              state.snapshot.torrents.map((torrent) => (
                <tr
                  className={torrent.info_hash === state.selectedInfoHash ? "selected" : ""}
                  key={torrent.info_hash}
                  onClick={() => dispatch({ type: "select_torrent", infoHash: torrent.info_hash })}
                >
                  <td className="name-cell">{torrent.name}</td>
                  <td>{torrent.status}</td>
                  <td>{formatBytes(torrent.size_bytes)}</td>
                  <td>{formatPercent(torrent.progress)}</td>
                  <td>{formatRate(torrent.down_rate)}</td>
                  <td>{formatRate(torrent.up_rate)}</td>
                  <td>
                    {torrent.seeds}/{torrent.peers}
                  </td>
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>
    </section>
  );
}
