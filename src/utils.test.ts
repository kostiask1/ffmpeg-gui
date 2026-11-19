import { describe, expect, it } from "vitest";
import {
  dedupeIncomingPaths,
  formatDuration,
  mergeUniqueFiles,
  parentDirectory,
  totalDuration,
} from "./utils";
import type { AudioFileInfo } from "./types";

const file = (
  duration: number,
  path = "D:\\Музика\\доріжка.flac",
): AudioFileInfo => ({
  name: "доріжка.flac",
  path,
  duration,
  codec: "flac",
  sampleRate: 48000,
  channels: 2,
  size: 1024,
  volume: 1,
  muted: false,
  speaker: "Мовець 1",
  language: "uk",
});

describe("audio helpers", () => {
  it("formats long durations", () =>
    expect(formatDuration(3723)).toBe("1:02:03"));
  it("calculates operation durations", () => {
    const files = [file(10), file(20), file(15)];
    expect(totalDuration(files, "mix", "longest")).toBe(20);
    expect(totalDuration(files, "mix", "shortest")).toBe(10);
    expect(totalDuration(files, "mix", "first")).toBe(10);
    expect(totalDuration(files, "concatenate", "longest")).toBe(45);
    expect(totalDuration(files, "transcribe", "longest")).toBe(45);
  });
  it("retains speaker and language metadata on tracks", () => {
    const item = file(12);
    expect(item.speaker).toBe("Мовець 1");
    expect(item.language).toBe("uk");
  });
  it("deduplicates incoming drop batches, existing files, and pending paths", () => {
    const existing = [file(10, "D:\\Audio\\Track1.flac")];
    const pending = new Set(["d:\\audio\\track2.flac"]);
    const incoming = [
      "D:\\Audio\\track1.flac",
      "D:\\Audio\\Track2.flac",
      "D:\\Audio\\Track3.flac",
      "d:\\audio\\track3.flac",
    ];
    expect(dedupeIncomingPaths(incoming, existing, pending)).toEqual([
      "D:\\Audio\\Track3.flac",
    ]);
  });
  it("merges probed files without duplicates and assigns speakers", () => {
    const first = file(10, "D:\\Audio\\Track1.flac");
    const dup = file(10, "d:\\audio\\track1.flac");
    const second = { ...file(20, "D:\\Audio\\Track2.flac"), speaker: "" };
    const merged = mergeUniqueFiles([first], [dup, second], "en");
    expect(merged).toHaveLength(2);
    expect(merged[1].speaker).toBe("Мовець 2");
    expect(merged[1].language).toBe("en");
  });
  it("handles Cyrillic Windows paths", () =>
    expect(parentDirectory("D:\\Музика\\Сцена 01\\доріжка №1.flac")).toBe(
      "D:\\Музика\\Сцена 01",
    ));
});
