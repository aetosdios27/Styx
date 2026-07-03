import { describe, expect, it } from "vitest";
import { formatBytes, formatPercent, formatRate } from "./format";

describe("formatters", () => {
  it("formats binary byte units", () => {
    expect(formatBytes(1536)).toBe("1.5 KiB");
  });

  it("formats rates with per-second suffix", () => {
    expect(formatRate(2_097_152)).toBe("2.0 MiB/s");
  });

  it("clamps progress percentages", () => {
    expect(formatPercent(1.4)).toBe("100.0%");
    expect(formatPercent(Number.NaN)).toBe("0.0%");
  });
});
