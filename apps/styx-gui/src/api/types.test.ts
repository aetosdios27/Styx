import { describe, expect, it } from "vitest";
import { emptySnapshot, isSnapshotEvent } from "./types";

describe("api types", () => {
  it("creates an empty snapshot matching the Rust default shape", () => {
    expect(emptySnapshot().totals.torrent_count).toBe(0);
  });

  it("narrows snapshot events", () => {
    expect(isSnapshotEvent({ type: "snapshot", snapshot: emptySnapshot() })).toBe(true);
  });
});
