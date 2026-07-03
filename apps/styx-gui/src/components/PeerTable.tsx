import { formatPercent, formatRate } from "../lib/format";
import { useAppState } from "../state/store";

export function PeerTable() {
  const state = useAppState();

  return (
    <section className="panel full-panel">
      <div className="panel-heading">
        <h2>Peers</h2>
        <span>{state.snapshot.peers.length}</span>
      </div>
      <div className="table-wrap">
        <table>
          <thead>
            <tr>
              <th>Address</th>
              <th>Flags</th>
              <th>Progress</th>
              <th>Down</th>
              <th>Up</th>
            </tr>
          </thead>
          <tbody>
            {state.snapshot.peers.length === 0 ? (
              <tr>
                <td colSpan={5} className="empty-cell">
                  No connected peers.
                </td>
              </tr>
            ) : (
              state.snapshot.peers.map((peer) => (
                <tr key={`${peer.torrent}-${peer.address}`}>
                  <td>{peer.address}</td>
                  <td>{peer.flags}</td>
                  <td>{formatPercent(peer.progress)}</td>
                  <td>{formatRate(peer.down_rate)}</td>
                  <td>{formatRate(peer.up_rate)}</td>
                </tr>
              ))
            )}
          </tbody>
        </table>
      </div>
    </section>
  );
}
