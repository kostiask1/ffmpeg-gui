import type {
  AudioFileInfo,
  MixDuration,
  Operation,
  TranscriptionLanguage,
} from "./types";

export function formatDuration(seconds: number): string {
  if (!Number.isFinite(seconds) || seconds < 0) return "—";
  const value = Math.round(seconds);
  const h = Math.floor(value / 3600);
  const m = Math.floor((value % 3600) / 60);
  const s = value % 60;
  return h > 0
    ? `${h}:${m.toString().padStart(2, "0")}:${s.toString().padStart(2, "0")}`
    : `${m}:${s.toString().padStart(2, "0")}`;
}

export function formatSize(bytes: number): string {
  if (!Number.isFinite(bytes)) return "—";
  const units = ["B", "KB", "MB", "GB", "TB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  return `${value.toFixed(unit < 2 ? 0 : 1)} ${units[unit]}`;
}

export function totalDuration(
  files: AudioFileInfo[],
  operation: Operation,
  mix: MixDuration,
): number {
  if (!files.length) return 0;
  if (operation === "concatenate" || operation === "transcribe")
    return files.reduce((sum, file) => sum + file.duration, 0);
  if (mix === "shortest")
    return Math.min(...files.map((file) => file.duration));
  if (mix === "first") return files[0].duration;
  return Math.max(...files.map((file) => file.duration));
}

export function parentDirectory(path: string): string {
  const slash = Math.max(path.lastIndexOf("\\"), path.lastIndexOf("/"));
  return slash >= 0 ? path.slice(0, slash) : "";
}

export function quoteCommandPart(value: string): string {
  return /[\s"&|<>^]/.test(value) ? `"${value.replace(/"/g, '\\"')}"` : value;
}

export function dedupeIncomingPaths(
  paths: string[],
  existing: AudioFileInfo[],
  pending: ReadonlySet<string>,
): string[] {
  const seen = new Set<string>(
    existing.map((file) => file.path.toLocaleLowerCase()),
  );
  for (const item of pending) {
    seen.add(item.toLocaleLowerCase());
  }
  const unique: string[] = [];
  for (const raw of paths) {
    const trimmed = raw.trim();
    if (!trimmed) continue;
    const key = trimmed.toLocaleLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    unique.push(trimmed);
  }
  return unique;
}

export function mergeUniqueFiles(
  current: AudioFileInfo[],
  added: AudioFileInfo[],
  defaultLanguage: TranscriptionLanguage,
): AudioFileInfo[] {
  const seen = new Set(current.map((file) => file.path.toLocaleLowerCase()));
  const next = [...current];
  for (const item of added) {
    const key = item.path.toLocaleLowerCase();
    if (seen.has(key)) continue;
    seen.add(key);
    next.push({
      ...item,
      speaker: item.speaker || `Мовець ${next.length + 1}`,
      language: defaultLanguage,
    });
  }
  return next;
}
