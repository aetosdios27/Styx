import { AppEvent, AppSnapshot, emptySnapshot, isSnapshotEvent } from "../api/types";

export interface AppState {
  snapshot: AppSnapshot;
  selectedInfoHash: string | null;
  activeView: "torrents" | "peers" | "logs" | "settings" | "stats";
  lastError: string | null;
}

export type AppAction =
  | { type: "snapshot_received"; snapshot: AppSnapshot }
  | { type: "event_received"; event: AppEvent }
  | { type: "select_torrent"; infoHash: string | null }
  | { type: "select_view"; view: AppState["activeView"] }
  | { type: "command_failed"; error: string };

export function initialState(): AppState {
  return {
    snapshot: emptySnapshot(),
    selectedInfoHash: null,
    activeView: "torrents",
    lastError: null
  };
}

export function reducer(state: AppState, action: AppAction): AppState {
  switch (action.type) {
    case "snapshot_received":
      return applySnapshot(state, action.snapshot);
    case "event_received":
      if (isSnapshotEvent(action.event)) {
        return applySnapshot(state, action.event.snapshot);
      }
      if (action.event.type === "command_failed") {
        return { ...state, lastError: action.event.error };
      }
      return state;
    case "select_torrent":
      return { ...state, selectedInfoHash: action.infoHash };
    case "select_view":
      return { ...state, activeView: action.view };
    case "command_failed":
      return { ...state, lastError: action.error };
  }
}

function applySnapshot(state: AppState, snapshot: AppSnapshot): AppState {
  const selectedStillExists = snapshot.torrents.some(
    (torrent) => torrent.info_hash === state.selectedInfoHash
  );
  return {
    ...state,
    snapshot,
    selectedInfoHash: selectedStillExists ? state.selectedInfoHash : snapshot.torrents[0]?.info_hash ?? null
  };
}
