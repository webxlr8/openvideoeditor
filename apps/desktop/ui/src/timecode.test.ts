import { describe, expect, it } from "vitest";
import { formatShort, formatTimecode, parseTimecode } from "./timecode";

describe("timecode", () => {
  it("formats HH:MM:SS:FF", () => {
    expect(formatTimecode(0, 30)).toBe("00:00:00:00");
    expect(formatTimecode(29, 30)).toBe("00:00:00:29");
    expect(formatTimecode(30 * 3661 + 5, 30)).toBe("01:01:01:05");
    expect(formatTimecode(24, 23.976)).toBe("00:00:01:00");
  });

  it("formats short", () => {
    expect(formatShort(1800, 30)).toBe("1:00");
    expect(formatShort(30 * 3725, 30)).toBe("1:02:05");
  });

  it("parses the accepted forms", () => {
    expect(parseTimecode("00:00:02:10", 30, 1000)).toBe(70);
    expect(parseTimecode("00:02:10", 30, 1000)).toBe(70);
    expect(parseTimecode("0:02", 30, 1000)).toBe(60);
    expect(parseTimecode("2.5", 30, 1000)).toBe(75);
    expect(parseTimecode("123f", 30, 1000)).toBe(123);
  });

  it("clamps to the media and rejects garbage", () => {
    expect(parseTimecode("99:00:00:00", 30, 1000)).toBe(999);
    expect(parseTimecode("abc", 30, 1000)).toBeNull();
    expect(parseTimecode("00:00:00:30", 30, 1000)).toBeNull();
    expect(parseTimecode("00:61", 30, 1000)).toBeNull();
    expect(parseTimecode("", 30, 1000)).toBeNull();
  });

  it("round-trips", () => {
    for (const f of [0, 1, 29, 30, 1799, 107999]) {
      expect(parseTimecode(formatTimecode(f, 30), 30, 200000)).toBe(f);
    }
  });
});
