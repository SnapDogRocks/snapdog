import { describe, expect, it } from "vitest";
import { formatTime } from "./format-time";

describe("formatTime", () => {
  it("formats zero as 0:00", () => {
    expect(formatTime(0)).toBe("0:00");
  });

  it("pads seconds under 10 with a leading zero", () => {
    expect(formatTime(5_000)).toBe("0:05");
  });

  it("formats whole minutes", () => {
    expect(formatTime(60_000)).toBe("1:00");
  });

  it("formats minutes and seconds together", () => {
    expect(formatTime(185_000)).toBe("3:05");
  });

  it("does not pad minutes past 9", () => {
    expect(formatTime(600_000)).toBe("10:00");
  });

  it("floors fractional seconds", () => {
    expect(formatTime(1_999)).toBe("0:01");
  });

  it("clamps negative durations to 0:00", () => {
    expect(formatTime(-500)).toBe("0:00");
  });
});
