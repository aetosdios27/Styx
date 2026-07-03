import { FolderPlus, Pause, Play, Trash2 } from "lucide-react";
import { addTorrent, pauseTorrent, removeTorrent, resumeTorrent } from "../api/client";
import { useAppDispatch, useAppState } from "../state/store";

export function Toolbar() {
  const state = useAppState();
  const dispatch = useAppDispatch();
  const selected = state.snapshot.torrents.find((torrent) => torrent.info_hash === state.selectedInfoHash);

  async function run(label: string, command: () => Promise<unknown>) {
    try {
      await command();
    } catch (error) {
      dispatch({ type: "command_failed", error: `${label}: ${String(error)}` });
    }
  }

  return (
    <header className="toolbar">
      <div>
        <h1>Torrent Operations</h1>
        <p>Command and inspect the local Styx session.</p>
      </div>
      <div className="toolbar-actions">
        <button
          aria-label="Add torrent"
          className="icon-button primary"
          onClick={() => run("add torrent", () => addTorrent("", null))}
          title="Add torrent"
          type="button"
        >
          <FolderPlus size={18} />
        </button>
        <button
          aria-label="Resume torrent"
          className="icon-button"
          disabled={!selected}
          onClick={() => selected && run("resume torrent", () => resumeTorrent(selected.info_hash))}
          title="Resume"
          type="button"
        >
          <Play size={18} />
        </button>
        <button
          aria-label="Pause torrent"
          className="icon-button"
          disabled={!selected}
          onClick={() => selected && run("pause torrent", () => pauseTorrent(selected.info_hash))}
          title="Pause"
          type="button"
        >
          <Pause size={18} />
        </button>
        <button
          aria-label="Remove torrent"
          className="icon-button danger"
          disabled={!selected}
          onClick={() => selected && run("remove torrent", () => removeTorrent(selected.info_hash))}
          title="Remove"
          type="button"
        >
          <Trash2 size={18} />
        </button>
      </div>
    </header>
  );
}
