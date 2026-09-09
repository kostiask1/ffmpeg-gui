import { useCallback, useEffect, useMemo, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebviewWindow } from "@tauri-apps/api/webviewWindow";
import { open } from "@tauri-apps/plugin-dialog";
import { openPath } from "@tauri-apps/plugin-opener";
import {
  AlertCircle,
  ArrowDown,
  ArrowUp,
  AudioLines,
  Check,
  ChevronDown,
  CircleStop,
  FileAudio,
  FolderOpen,
  GripVertical,
  ListMusic,
  LoaderCircle,
  Plus,
  RefreshCw,
  Settings,
  SlidersHorizontal,
  Trash2,
  X,
} from "lucide-react";
import type {
  AppSettings,
  AudioFileInfo,
  ConcatMode,
  FFmpegStatus,
  MixDuration,
  OutputFormat,
  ProcessingFailed,
  ProcessingFinished,
  ProcessingJob,
  ProcessingProgress,
  ProcessingStarted,
  WavDepth,
} from "./types";
import {
  formatDuration,
  formatSize,
  parentDirectory,
  quoteCommandPart,
  totalDuration,
} from "./utils";

const defaultSettings: AppSettings = {
  ffmpegPath: null,
  openOutputFolder: false,
  showCommand: false,
  rememberLastOutputFolder: true,
  lastOutputFolder: null,
};

type RunState = "idle" | "running" | "success" | "failed" | "cancelled";

function App() {
  const [files, setFiles] = useState<AudioFileInfo[]>([]);
  const [probing, setProbing] = useState(false);
  const [probeError, setProbeError] = useState("");
  const [ffmpeg, setFfmpeg] = useState<FFmpegStatus>({ found: false });
  const [settings, setSettings] = useState(defaultSettings);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [operation, setOperation] = useState<"mix" | "concatenate">("mix");
  const [mixDuration, setMixDuration] = useState<MixDuration>("longest");
  const [concatMode, setConcatMode] = useState<ConcatMode>("auto");
  const [format, setFormat] = useState<OutputFormat>("flac");
  const [flacCompression, setFlacCompression] = useState(8);
  const [wavCodec, setWavCodec] = useState<WavDepth>("pcm_s24le");
  const [sampleRate, setSampleRate] = useState<number | null>(null);
  const [channels, setChannels] = useState<number | null>(null);
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [outputName, setOutputName] = useState("combined");
  const [outputDirectory, setOutputDirectory] = useState("");
  const [runState, setRunState] = useState<RunState>("idle");
  const [progress, setProgress] = useState<ProcessingProgress | null>(null);
  const [runningCommand, setRunningCommand] = useState<string[]>([]);
  const [result, setResult] = useState<ProcessingFinished | null>(null);
  const [failure, setFailure] = useState<ProcessingFailed | null>(null);
  const [overwriteJob, setOverwriteJob] = useState<ProcessingJob | null>(null);
  const [dragIndex, setDragIndex] = useState<number | null>(null);

  const refreshFfmpeg = useCallback(async () => {
    setFfmpeg(await invoke<FFmpegStatus>("detect_ffmpeg"));
  }, []);

  useEffect(() => {
    void Promise.all([
      invoke<AppSettings>("load_settings").then(setSettings),
      refreshFfmpeg(),
    ]);
  }, [refreshFfmpeg]);

  const addPaths = useCallback(
    async (paths: string[]) => {
      const unique = paths.filter(
        (path) =>
          !files.some(
            (file) =>
              file.path.toLocaleLowerCase() === path.toLocaleLowerCase(),
          ),
      );
      if (!unique.length) return;
      setProbing(true);
      setProbeError("");
      try {
        const added = await invoke<AudioFileInfo[]>("probe_files", {
          paths: unique,
        });
        setFiles((current) => [...current, ...added]);
        if (!outputDirectory && added[0]) {
          const remembered = settings.rememberLastOutputFolder
            ? settings.lastOutputFolder
            : null;
          setOutputDirectory(remembered || parentDirectory(added[0].path));
        }
      } catch (error) {
        setProbeError(String(error));
      } finally {
        setProbing(false);
      }
    },
    [files, outputDirectory, settings],
  );

  useEffect(() => {
    const unlisteners: Array<() => void> = [];
    void getCurrentWebviewWindow()
      .onDragDropEvent((event) => {
        if (event.payload.type === "drop") void addPaths(event.payload.paths);
      })
      .then((unlisten) => unlisteners.push(unlisten));
    void listen<ProcessingProgress>("processing-progress", ({ payload }) => {
      setProgress(payload);
    }).then((unlisten) => unlisteners.push(unlisten));
    void listen<ProcessingFinished>("processing-finished", ({ payload }) => {
      setRunState("success");
      setResult(payload);
      setProgress(
        (current) =>
          current && {
            ...current,
            percent: 100,
            processedSeconds: current.totalSeconds,
          },
      );
      if (settings.openOutputFolder)
        void openPath(parentDirectory(payload.outputPath));
    }).then((unlisten) => unlisteners.push(unlisten));
    void listen<ProcessingFailed>("processing-failed", ({ payload }) => {
      setRunState(payload.cancelled ? "cancelled" : "failed");
      setFailure(payload.cancelled ? null : payload);
    }).then((unlisten) => unlisteners.push(unlisten));
    void listen("close-requested-during-processing", () => {
      if (
        window.confirm(
          "Audio processing is still running. Cancel it and close the application?",
        )
      ) {
        void invoke("request_app_exit");
      }
    }).then((unlisten) => unlisteners.push(unlisten));
    return () => unlisteners.forEach((unlisten) => unlisten());
  }, [addPaths, settings.openOutputFolder]);

  const chooseFiles = async () => {
    const selected = await open({
      multiple: true,
      filters: [
        {
          name: "Audio",
          extensions: [
            "flac",
            "wav",
            "mp3",
            "m4a",
            "aac",
            "ogg",
            "opus",
            "wma",
          ],
        },
      ],
    });
    if (selected)
      await addPaths(Array.isArray(selected) ? selected : [selected]);
  };

  const chooseFolder = async () => {
    const selected = await open({
      directory: true,
      multiple: false,
      defaultPath: outputDirectory || undefined,
    });
    if (selected) setOutputDirectory(selected);
  };

  const buildJob = (replaceExisting = false): ProcessingJob => ({
    id: crypto.randomUUID(),
    inputs: files,
    operation,
    mix: { duration: mixDuration },
    concat: { mode: concatMode },
    encoding: { format, flacCompression, wavCodec, sampleRate, channels },
    outputDirectory,
    outputName: outputName.trim(),
    replaceExisting,
  });

  const runJob = async (job: ProcessingJob) => {
    setOverwriteJob(null);
    setRunState("running");
    setProgress({
      jobId: job.id,
      processedSeconds: 0,
      totalSeconds: totalDuration(files, operation, mixDuration),
      percent: 0,
      elapsedSeconds: 0,
    });
    setFailure(null);
    setResult(null);
    setRunningCommand([]);
    try {
      const started = await invoke<ProcessingStarted>("start_processing", {
        job,
      });
      setRunningCommand(started.command);
      if (
        settings.rememberLastOutputFolder &&
        settings.lastOutputFolder !== outputDirectory
      ) {
        const next = { ...settings, lastOutputFolder: outputDirectory };
        setSettings(next);
        await invoke("save_settings", { value: next });
      }
    } catch (error) {
      setRunState("failed");
      setFailure({
        jobId: job.id,
        message: String(error),
        log: "",
        command: [],
        cancelled: false,
      });
    }
  };

  const start = async () => {
    const job = buildJob();
    if (await invoke<boolean>("destination_exists", { job }))
      setOverwriteJob(job);
    else await runJob(job);
  };

  const cancel = async () => {
    await invoke("cancel_processing");
  };

  const valid =
    ffmpeg.found &&
    files.length > 0 &&
    outputName.trim().length > 0 &&
    outputDirectory.length > 0 &&
    !outputName.includes("/") &&
    !outputName.includes("\\");
  const command = result?.command ?? failure?.command ?? runningCommand;

  const move = (from: number, to: number) => {
    if (to < 0 || to >= files.length || from === to) return;
    setFiles((current) => {
      const next = [...current];
      const [item] = next.splice(from, 1);
      next.splice(to, 0, item);
      return next;
    });
  };

  return (
    <div className="app-shell">
      <header className="topbar">
        <div className="brand">
          <span className="brand-mark">
            <AudioLines size={23} />
          </span>
          <div>
            <strong>Audio Forge</strong>
            <small>Local FFmpeg audio workspace</small>
          </div>
        </div>
        <div className="top-actions">
          <span className={`status-pill ${ffmpeg.found ? "ready" : "missing"}`}>
            <span />
            {ffmpeg.found ? "FFmpeg ready" : "FFmpeg not found"}
          </span>
          <button
            className="icon-button"
            onClick={() => setSettingsOpen(true)}
            aria-label="Settings"
          >
            <Settings size={19} />
          </button>
        </div>
      </header>

      <main>
        {!ffmpeg.found && (
          <div className="ffmpeg-warning">
            <AlertCircle />
            <div>
              <strong>FFmpeg was not found</strong>
              <span>Install it from PATH or select ffmpeg.exe manually.</span>
            </div>
            <button className="button secondary" onClick={refreshFfmpeg}>
              <RefreshCw size={16} />
              Check again
            </button>
            <button
              className="button primary"
              onClick={() => setSettingsOpen(true)}
            >
              Select executable
            </button>
          </div>
        )}

        <section className="panel input-panel">
          <div className="section-heading">
            <div>
              <span className="eyebrow">01 · SOURCES</span>
              <h2>
                Input files <em>{files.length}</em>
              </h2>
            </div>
            {files.length > 0 && (
              <button
                className="text-button danger"
                disabled={runState === "running"}
                onClick={() => setFiles([])}
              >
                <Trash2 size={15} />
                Clear all
              </button>
            )}
          </div>
          {files.length === 0 ? (
            <div className="drop-zone" onClick={chooseFiles}>
              <div className="drop-icon">
                <FileAudio />
              </div>
              <h3>Drop audio files here</h3>
              <p>FLAC, WAV, MP3, AAC, Opus and other FFmpeg formats</p>
              <button className="button primary" type="button">
                <Plus size={17} />
                Add files
              </button>
            </div>
          ) : (
            <>
              <div className="file-table-head">
                <span>Name</span>
                <span>Duration</span>
                <span>Format</span>
                <span>Sample rate</span>
                <span>Channels</span>
                <span>Size</span>
                <span />
              </div>
              <div className="file-list">
                {files.map((file, index) => (
                  <div
                    className={`file-row ${dragIndex === index ? "dragging" : ""}`}
                    key={file.path}
                    draggable={runState !== "running"}
                    onDragStart={() => setDragIndex(index)}
                    onDragOver={(e) => e.preventDefault()}
                    onDrop={() => {
                      if (dragIndex !== null) move(dragIndex, index);
                      setDragIndex(null);
                    }}
                    onDragEnd={() => setDragIndex(null)}
                  >
                    <div className="file-name">
                      <GripVertical className="grip" size={17} />
                      <span className="file-type">
                        <FileAudio size={18} />
                      </span>
                      <div>
                        <strong title={file.path}>{file.name}</strong>
                        <small title={file.path}>{file.path}</small>
                      </div>
                    </div>
                    <span>{formatDuration(file.duration)}</span>
                    <span className="codec-badge">
                      {file.codec.toUpperCase()}
                    </span>
                    <span>
                      {file.sampleRate ? `${file.sampleRate / 1000} kHz` : "—"}
                    </span>
                    <span>
                      {file.channels === 1
                        ? "Mono"
                        : file.channels === 2
                          ? "Stereo"
                          : file.channelLayout || file.channels || "—"}
                    </span>
                    <span>{formatSize(file.size)}</span>
                    <div className="row-actions">
                      <button
                        onClick={() => move(index, index - 1)}
                        disabled={index === 0 || runState === "running"}
                        aria-label="Move up"
                      >
                        <ArrowUp />
                      </button>
                      <button
                        onClick={() => move(index, index + 1)}
                        disabled={
                          index === files.length - 1 || runState === "running"
                        }
                        aria-label="Move down"
                      >
                        <ArrowDown />
                      </button>
                      <button
                        onClick={() =>
                          setFiles((current) =>
                            current.filter((_, i) => i !== index),
                          )
                        }
                        disabled={runState === "running"}
                        aria-label="Remove"
                      >
                        <X />
                      </button>
                    </div>
                  </div>
                ))}
              </div>
              <button
                className="add-more"
                disabled={probing || runState === "running"}
                onClick={chooseFiles}
              >
                {probing ? <LoaderCircle className="spin" /> : <Plus />}{" "}
                {probing ? "Reading metadata…" : "Add more files"}
              </button>
            </>
          )}
          {probeError && (
            <div className="inline-error">
              <AlertCircle size={16} />
              {probeError}
            </div>
          )}
        </section>

        <div className="work-grid">
          <section className="panel">
            <div className="section-heading">
              <div>
                <span className="eyebrow">02 · METHOD</span>
                <h2>Operation</h2>
              </div>
            </div>
            <div className="operation-cards">
              <button
                className={operation === "mix" ? "selected" : ""}
                onClick={() => setOperation("mix")}
                disabled={runState === "running"}
              >
                <span className="radio-dot" />
                <AudioLines />
                <div>
                  <strong>Mix / Overlay</strong>
                  <small>Play every track together from 0:00</small>
                </div>
              </button>
              <button
                className={operation === "concatenate" ? "selected" : ""}
                onClick={() => setOperation("concatenate")}
                disabled={runState === "running"}
              >
                <span className="radio-dot" />
                <ListMusic />
                <div>
                  <strong>Concatenate</strong>
                  <small>Join tracks in the order shown above</small>
                </div>
              </button>
            </div>
            <div className="field operation-setting">
              {operation === "mix" ? (
                <>
                  <label>Mix duration</label>
                  <select
                    value={mixDuration}
                    onChange={(e) =>
                      setMixDuration(e.target.value as MixDuration)
                    }
                  >
                    <option value="longest">Longest track</option>
                    <option value="shortest">Shortest track</option>
                    <option value="first">First track</option>
                  </select>
                </>
              ) : (
                <>
                  <label>Concatenation mode</label>
                  <select
                    value={concatMode}
                    onChange={(e) =>
                      setConcatMode(e.target.value as ConcatMode)
                    }
                  >
                    <option value="auto">Auto — copy when safe</option>
                    <option value="copy">Stream Copy</option>
                    <option value="reencode">Re-encode</option>
                  </select>
                  <small>
                    Auto uses ffprobe metadata and copies only compatible FLAC
                    streams.
                  </small>
                </>
              )}
            </div>
          </section>

          <section className="panel">
            <div className="section-heading">
              <div>
                <span className="eyebrow">03 · DESTINATION</span>
                <h2>Output</h2>
              </div>
            </div>
            <div className="output-grid">
              <div className="field">
                <label>Format</label>
                <select
                  value={format}
                  onChange={(e) => setFormat(e.target.value as OutputFormat)}
                  disabled={runState === "running"}
                >
                  <option value="flac">FLAC</option>
                  <option value="wav">WAV</option>
                </select>
              </div>
              <div className="field filename">
                <label>Filename</label>
                <div className="suffix-input">
                  <input
                    value={outputName}
                    onChange={(e) => setOutputName(e.target.value)}
                    disabled={runState === "running"}
                  />
                  <span>.{format}</span>
                </div>
              </div>
            </div>
            <div className="field">
              <label>Output folder</label>
              <div className="folder-input">
                <input
                  value={outputDirectory}
                  readOnly
                  placeholder="Select an output folder"
                />
                <button
                  onClick={chooseFolder}
                  disabled={runState === "running"}
                >
                  <FolderOpen />
                </button>
              </div>
            </div>
            {format === "flac" ? (
              <div className="field compression">
                <div className="label-line">
                  <label>FLAC compression</label>
                  <output>{flacCompression}</output>
                </div>
                <input
                  type="range"
                  min="0"
                  max="12"
                  value={flacCompression}
                  onChange={(e) => setFlacCompression(Number(e.target.value))}
                  disabled={runState === "running"}
                />
                <small>
                  Affects encoding speed and file size, never audio quality.
                </small>
              </div>
            ) : (
              <div className="field">
                <label>WAV encoding</label>
                <select
                  value={wavCodec}
                  onChange={(e) => setWavCodec(e.target.value as WavDepth)}
                  disabled={runState === "running"}
                >
                  <option value="pcm_s16le">16-bit PCM</option>
                  <option value="pcm_s24le">24-bit PCM</option>
                  <option value="pcm_f32le">32-bit float</option>
                </select>
              </div>
            )}
          </section>
        </div>

        <section className="panel advanced">
          <button
            className="advanced-toggle"
            onClick={() => setAdvancedOpen(!advancedOpen)}
          >
            <SlidersHorizontal />
            <div>
              <strong>Advanced audio settings</strong>
              <small>Optional output resampling and channel layout</small>
            </div>
            <ChevronDown className={advancedOpen ? "open" : ""} />
          </button>
          {advancedOpen && (
            <div className="advanced-body">
              <div className="field">
                <label>Sample rate</label>
                <select
                  value={sampleRate ?? ""}
                  onChange={(e) =>
                    setSampleRate(
                      e.target.value ? Number(e.target.value) : null,
                    )
                  }
                >
                  <option value="">Auto</option>
                  <option value="44100">44,100 Hz</option>
                  <option value="48000">48,000 Hz</option>
                  <option value="96000">96,000 Hz</option>
                </select>
              </div>
              <div className="field">
                <label>Channels</label>
                <select
                  value={channels ?? ""}
                  onChange={(e) =>
                    setChannels(e.target.value ? Number(e.target.value) : null)
                  }
                >
                  <option value="">Auto</option>
                  <option value="1">Mono</option>
                  <option value="2">Stereo</option>
                </select>
              </div>
            </div>
          )}
        </section>

        {(runState !== "idle" || failure) && (
          <section className={`panel progress-panel ${runState}`}>
            <div className="progress-title">
              <div className="progress-state-icon">
                {runState === "running" ? (
                  <LoaderCircle className="spin" />
                ) : runState === "success" ? (
                  <Check />
                ) : runState === "failed" ? (
                  <AlertCircle />
                ) : (
                  <CircleStop />
                )}
              </div>
              <div>
                <span className="eyebrow">PROCESSING STATUS</span>
                <h2>
                  {runState === "running"
                    ? "Processing audio…"
                    : runState === "success"
                      ? "Processing complete"
                      : runState === "cancelled"
                        ? "Processing cancelled"
                        : "Processing failed"}
                </h2>
              </div>
            </div>
            {progress && (
              <>
                <div className="progress-numbers">
                  <strong>{progress.percent.toFixed(1)}%</strong>
                  <span>
                    {formatDuration(progress.processedSeconds)} /{" "}
                    {formatDuration(progress.totalSeconds)}
                  </span>
                </div>
                <div className="progress-track">
                  <div style={{ width: `${progress.percent}%` }} />
                </div>
                <div className="progress-meta">
                  <span>
                    Elapsed{" "}
                    <strong>{formatDuration(progress.elapsedSeconds)}</strong>
                  </span>
                  <span>
                    Remaining{" "}
                    <strong>
                      {progress.remainingSeconds === undefined
                        ? "—"
                        : formatDuration(progress.remainingSeconds)}
                    </strong>
                  </span>
                  <span>
                    Speed{" "}
                    <strong>
                      {progress.speed ? `${progress.speed.toFixed(2)}×` : "—"}
                    </strong>
                  </span>
                </div>
              </>
            )}
            {result && (
              <p className="success-path" title={result.outputPath}>
                {result.usedStreamCopy ? "Stream copied" : "Encoded"} to{" "}
                {result.outputPath}
              </p>
            )}
            {failure && <ErrorDetails failure={failure} />}
            {command.length > 0 && settings.showCommand && (
              <CommandDetails command={command} />
            )}
          </section>
        )}

        <div className="action-bar">
          <div>
            <span>Estimated output length</span>
            <strong>
              {formatDuration(totalDuration(files, operation, mixDuration))}
            </strong>
          </div>
          {runState === "running" ? (
            <button className="button cancel" onClick={cancel}>
              <CircleStop />
              Cancel processing
            </button>
          ) : (
            <button
              className="button start"
              onClick={start}
              disabled={!valid || probing}
            >
              <AudioLines />
              Start processing
            </button>
          )}
        </div>
      </main>

      {settingsOpen && (
        <SettingsDialog
          settings={settings}
          ffmpeg={ffmpeg}
          onClose={() => setSettingsOpen(false)}
          onSave={async (next, nextStatus) => {
            setSettings(next);
            setFfmpeg(nextStatus);
            await invoke("save_settings", { value: next });
            setSettingsOpen(false);
          }}
        />
      )}
      {overwriteJob && (
        <div className="modal-backdrop">
          <div className="modal small">
            <div className="modal-heading">
              <AlertCircle />
              <div>
                <h2>Output already exists</h2>
                <p>
                  Choose how to continue. Audio Forge never overwrites a file
                  silently.
                </p>
              </div>
            </div>
            <div className="modal-actions triple">
              <button
                className="button danger-fill"
                onClick={() =>
                  runJob({
                    ...overwriteJob,
                    replaceExisting: true,
                    id: crypto.randomUUID(),
                  })
                }
              >
                Replace
              </button>
              <button
                className="button secondary"
                onClick={() => {
                  setOutputName(`${overwriteJob.outputName} (1)`);
                  setOverwriteJob(null);
                }}
              >
                Choose another name
              </button>
              <button
                className="button ghost"
                onClick={() => setOverwriteJob(null)}
              >
                Cancel
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  );
}

function ErrorDetails({ failure }: { failure: ProcessingFailed }) {
  const [openLog, setOpenLog] = useState(false);
  return (
    <div className="error-details">
      <strong>{failure.message}</strong>
      {failure.log && (
        <>
          <button className="text-button" onClick={() => setOpenLog(!openLog)}>
            <ChevronDown className={openLog ? "open" : ""} />
            FFmpeg log
          </button>
          {openLog && (
            <>
              <pre>{failure.log}</pre>
              <button
                className="button secondary compact"
                onClick={() => navigator.clipboard.writeText(failure.log)}
              >
                Copy log
              </button>
            </>
          )}
        </>
      )}
    </div>
  );
}

function CommandDetails({ command }: { command: string[] }) {
  const [openCommand, setOpenCommand] = useState(false);
  const value = useMemo(
    () => command.map(quoteCommandPart).join(" "),
    [command],
  );
  return (
    <div className="command-details">
      <button
        className="text-button"
        onClick={() => setOpenCommand(!openCommand)}
      >
        <ChevronDown className={openCommand ? "open" : ""} />
        Exact FFmpeg command
      </button>
      {openCommand && (
        <>
          <pre>{value}</pre>
          <button
            className="button secondary compact"
            onClick={() => navigator.clipboard.writeText(value)}
          >
            Copy FFmpeg command
          </button>
        </>
      )}
    </div>
  );
}

function SettingsDialog({
  settings,
  ffmpeg,
  onClose,
  onSave,
}: {
  settings: AppSettings;
  ffmpeg: FFmpegStatus;
  onClose: () => void;
  onSave: (settings: AppSettings, status: FFmpegStatus) => Promise<void>;
}) {
  const [draft, setDraft] = useState(settings);
  const [status, setStatus] = useState(ffmpeg);
  const [checking, setChecking] = useState(false);
  const check = async (path: string | null) => {
    setChecking(true);
    try {
      setStatus(
        path
          ? await invoke("validate_ffmpeg_path", { path })
          : await invoke("validate_ffmpeg_path", { path: "ffmpeg.exe" }),
      );
    } finally {
      setChecking(false);
    }
  };
  const browse = async () => {
    const selected = await open({
      multiple: false,
      filters: [{ name: "FFmpeg executable", extensions: ["exe"] }],
    });
    if (selected) {
      setDraft((current) => ({ ...current, ffmpegPath: selected }));
      await check(selected);
    }
  };
  return (
    <div className="modal-backdrop">
      <div className="modal settings-modal">
        <div className="modal-top">
          <div>
            <span className="eyebrow">APPLICATION</span>
            <h2>Settings</h2>
          </div>
          <button className="icon-button" onClick={onClose}>
            <X />
          </button>
        </div>
        <div className="settings-section">
          <label>FFmpeg path</label>
          <div className="path-row">
            <input
              value={draft.ffmpegPath ?? "Auto detected from PATH"}
              readOnly
              className={!draft.ffmpegPath ? "muted-value" : ""}
            />
            <button className="button secondary" onClick={browse}>
              <FolderOpen />
              Browse
            </button>
          </div>
          <div className={`detection ${status.found ? "ok" : "bad"}`}>
            {status.found ? <Check /> : <AlertCircle />}
            <span>
              <strong>
                {status.found
                  ? "FFmpeg detected"
                  : status.error || "FFmpeg was not found"}
              </strong>
              <small>{status.version || status.ffmpegPath}</small>
            </span>
            <button
              className="text-button"
              onClick={() => check(draft.ffmpegPath)}
              disabled={checking}
            >
              <RefreshCw className={checking ? "spin" : ""} />
              Auto Detect
            </button>
          </div>
          {draft.ffmpegPath && (
            <button
              className="text-button"
              onClick={() => {
                setDraft((current) => ({ ...current, ffmpegPath: null }));
                void check(null);
              }}
            >
              Use PATH auto detection
            </button>
          )}
        </div>
        <div className="settings-section checks">
          <label>
            <input
              type="checkbox"
              checked={draft.openOutputFolder}
              onChange={(e) =>
                setDraft({ ...draft, openOutputFolder: e.target.checked })
              }
            />
            <span>Open output folder after successful processing</span>
          </label>
          <label>
            <input
              type="checkbox"
              checked={draft.showCommand}
              onChange={(e) =>
                setDraft({ ...draft, showCommand: e.target.checked })
              }
            />
            <span>Show FFmpeg command</span>
          </label>
          <label>
            <input
              type="checkbox"
              checked={draft.rememberLastOutputFolder}
              onChange={(e) =>
                setDraft({
                  ...draft,
                  rememberLastOutputFolder: e.target.checked,
                })
              }
            />
            <span>Remember last output folder</span>
          </label>
        </div>
        <div className="modal-actions">
          <button className="button ghost" onClick={onClose}>
            Cancel
          </button>
          <button
            className="button primary"
            disabled={!status.found}
            onClick={() => onSave(draft, status)}
          >
            Save settings
          </button>
        </div>
      </div>
    </div>
  );
}

export default App;
