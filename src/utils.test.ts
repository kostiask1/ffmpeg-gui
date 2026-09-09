import { describe, expect, it } from "vitest";
import { formatDuration, parentDirectory, totalDuration } from "./utils";
import type { AudioFileInfo } from "./types";

const file = (duration: number): AudioFileInfo => ({
  name: "доріжка.flac",
  path: "D:\\Музика\\доріжка.flac",
  duration,
  codec: "flac",
  sampleRate: 48000,
  channels: 2,
  size: 1024,
  volume: 1,
  muted: false,
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
  });
  it("handles Cyrillic Windows paths", () =>
    expect(parentDirectory("D:\\Музика\\Сцена 01\\доріжка №1.flac")).toBe(
      "D:\\Музика\\Сцена 01",
    ));
});
