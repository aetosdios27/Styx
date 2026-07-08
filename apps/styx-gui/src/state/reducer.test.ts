import { describe, expect, it } from "vitest";
import { AppSnapshot } from "../api/types";
import { initialState, reducer } from "./reducer";

const snapshot: AppSnapshot = {
  torrents: [
    {
      info_hash: "1111111111111111111111111111111111111111",
      name: "ubuntu.iso",
      status: "checking",
      size_bytes: 1024,
      progress: 0.25,
      uploaded_bytes: 0,
      share_ratio: 0,
      down_rate: 0,
      up_rate: 0,
      peers: 0,
      seeds: 0
    }
  ],
  peers: [],
  speed: [],
  logs: [],
  totals: {
    down_bytes: 0,
    up_bytes: 0,
    torrent_count: 1,
    peer_count: 0
  }
};

describe("reducer", () => {
  it("selects the first torrent when a snapshot arrives", () => {
    const state = reducer(initialState(), { type: "snapshot_received", snapshot });

    expect(state.selectedInfoHash).toBe("1111111111111111111111111111111111111111");
  });

  it("stores command failure text", () => {
    const state = reducer(initialState(), { type: "command_failed", error: "bad torrent" });

    expect(state.lastError).toBe("bad torrent");
  });
});
