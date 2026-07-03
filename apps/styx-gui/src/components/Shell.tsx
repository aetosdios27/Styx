import { Activity, BarChart3, Files, ListTree, Settings, TerminalSquare, Users } from "lucide-react";
import { LogsPanel } from "./LogsPanel";
import { PeerTable } from "./PeerTable";
import { SettingsPanel } from "./SettingsPanel";
import { StatsPanel } from "./StatsPanel";
import { Toolbar } from "./Toolbar";
import { TorrentDetail } from "./TorrentDetail";
import { TorrentTable } from "./TorrentTable";
import { useAppDispatch, useAppState } from "../state/store";

const nav = [
  { id: "torrents", label: "Torrents", icon: Files },
  { id: "peers", label: "Peers", icon: Users },
  { id: "logs", label: "Logs", icon: TerminalSquare },
  { id: "settings", label: "Settings", icon: Settings },
  { id: "stats", label: "Stats", icon: BarChart3 }
] as const;

export function Shell() {
  const state = useAppState();
  const dispatch = useAppDispatch();

  return (
    <div className="app-shell">
      <aside className="sidebar" aria-label="Primary navigation">
        <div className="brand">
          <Activity size={22} />
          <span>Styx</span>
        </div>
        <nav className="nav-list">
          {nav.map((item) => {
            const Icon = item.icon;
            return (
              <button
                className={state.activeView === item.id ? "nav-item active" : "nav-item"}
                key={item.id}
                onClick={() => dispatch({ type: "select_view", view: item.id })}
                type="button"
              >
                <Icon size={18} />
                <span>{item.label}</span>
              </button>
            );
          })}
        </nav>
      </aside>
      <main className="workspace">
        <Toolbar />
        {state.lastError ? <div className="error-strip">{state.lastError}</div> : null}
        {state.activeView === "torrents" ? (
          <section className="split-view">
            <TorrentTable />
            <TorrentDetail />
          </section>
        ) : null}
        {state.activeView === "peers" ? <PeerTable /> : null}
        {state.activeView === "logs" ? <LogsPanel /> : null}
        {state.activeView === "settings" ? <SettingsPanel /> : null}
        {state.activeView === "stats" ? <StatsPanel /> : null}
        <footer className="statusbar">
          <span>{state.snapshot.totals.torrent_count} torrents</span>
          <span>{state.snapshot.totals.peer_count} peers</span>
        </footer>
      </main>
    </div>
  );
}
