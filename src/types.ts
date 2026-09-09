export type Operation = "mix" | "concatenate" | "multichannel";
export type MixDuration = "longest" | "shortest" | "first";
export type ConcatMode = "auto" | "copy" | "reencode";
export type OutputFormat = "flac" | "wav";
export type WavDepth = "pcm_s16le" | "pcm_s24le" | "pcm_f32le";

export interface AudioFileInfo {
  name: string;
  path: string;
  duration: number;
  codec: string;
  sampleRate?: number;
  channels?: number;
  channelLayout?: string;
  sampleFormat?: string;
  bitDepth?: number;
  bitrate?: number;
  size: number;
  volume: number;
  muted: boolean;
}

export interface AppSettings {
  ffmpegPath: string | null;
  openOutputFolder: boolean;
  showCommand: boolean;
  rememberLastOutputFolder: boolean;
  lastOutputFolder: string | null;
}

export interface FFmpegStatus {
  found: boolean;
  source?: "bundled" | "custom" | "path";
  ffmpegPath?: string;
  ffprobePath?: string;
  version?: string;
  error?: string;
}

export interface ProcessingJob {
  id: string;
  inputs: AudioFileInfo[];
  operation: Operation;
  mix: { duration: MixDuration };
  concat: { mode: ConcatMode };
  encoding: {
    format: OutputFormat;
    flacCompression: number;
    wavCodec: WavDepth;
    sampleRate: number | null;
    channels: number | null;
  };
  outputDirectory: string;
  outputName: string;
  replaceExisting: boolean;
}

export interface ProcessingProgress {
  jobId: string;
  processedSeconds: number;
  totalSeconds: number;
  percent: number;
  elapsedSeconds: number;
  remainingSeconds?: number;
  speed?: number;
}

export interface ProcessingStarted {
  jobId: string;
  command: string[];
  usedStreamCopy: boolean;
}

export interface ProcessingFinished {
  jobId: string;
  outputPath: string;
  command: string[];
  usedStreamCopy: boolean;
}

export interface ProcessingFailed {
  jobId: string;
  message: string;
  log: string;
  command: string[];
  cancelled: boolean;
}
