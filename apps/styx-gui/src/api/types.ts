export type InfoHashHex = string;

export type TorrentStatus = "checking" | "paused" | "downloading" | "seeding" | "error";

export interface TorrentRow {
  info_hash: InfoHashHex;
  name: string;
  status: TorrentStatus;
  size_bytes: number;
  progress: number;
  down_rate: number;
  up_rate: number;
  peers: number;
  seeds: number;
}

export interface PeerRow {
  torrent: InfoHashHex;
  address: string;
  flags: string;
  progress: number;
  down_rate: number;
  up_rate: number;
}

export interface SpeedSample {
  second: number;
  down_rate: number;
  up_rate: number;
}

export type LogLevel = "info" | "warn" | "error";

export interface LogLine {
  level: LogLevel;
  message: string;
}

export interface SessionTotals {
  down_bytes: number;
  up_bytes: number;
  torrent_count: number;
  peer_count: number;
}

export interface AppSnapshot {
  torrents: TorrentRow[];
  peers: PeerRow[];
  speed: SpeedSample[];
  logs: LogLine[];
  totals: SessionTotals;
}

export type CommandResponse =
  | { type: "torrent_added"; info_hash: InfoHashHex; name: string }
  | { type: "torrent_removed"; info_hash: InfoHashHex }
  | { type: "torrent_paused"; info_hash: InfoHashHex }
  | { type: "torrent_resumed"; info_hash: InfoHashHex }
  | { type: "status"; snapshot: AppSnapshot };

export type AppEvent =
  | { type: "daemon_started"; ipc: string | null; at_ms: number }
  | { type: "torrent_added"; info_hash: InfoHashHex; name: string }
  | { type: "torrent_removed"; info_hash: InfoHashHex }
  | { type: "snapshot"; snapshot: AppSnapshot }
  | { type: "command_failed"; command: string; error: string };

export function emptySnapshot(): AppSnapshot {
  return {
    torrents: [],
    peers: [],
    speed: [],
    logs: [],
    totals: {
      down_bytes: 0,
      up_bytes: 0,
      torrent_count: 0,
      peer_count: 0
    }
  };
}

export function isSnapshotEvent(event: AppEvent): event is Extract<AppEvent, { type: "snapshot" }> {
  return event.type === "snapshot";
}
