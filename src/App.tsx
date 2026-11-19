import { useCallback, useEffect, useMemo, useRef, useState } from "react";
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
  FileText,
  FolderOpen,
  GripVertical,
  Languages,
  ListMusic,
  LoaderCircle,
  Mic,
  Plus,
  RefreshCw,
  Settings,
  SlidersHorizontal,
  Trash2,
  User,
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
  ProcessingLog,
  ProcessingProgress,
  ProcessingStarted,
  TranscriptFormat,
  TranscriptionLanguage,
  WavDepth,
} from "./types";
import {
  dedupeIncomingPaths,
  formatDuration,
  formatSize,
  mergeUniqueFiles,
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

const PARAKEET_MODEL_ID = "ihnatov-yuri/parakeet-tdt-0.6b-v3-uk";
const PARAKEET_ATTRIBUTION =
  "Yuri Ihnatov (https://ihnatov.nl), fine-tuned from NVIDIA Parakeet TDT 0.6B v3 (CC BY 4.0)";

const LANGUAGE_OPTIONS: Array<{
  value: TranscriptionLanguage;
  label: string;
}> = [
  { value: "uk", label: "Українська (UK)" },
  { value: "auto", label: "Авто (UK + EN)" },
  { value: "en", label: "English (EN)" },
  { value: "uk_strict", label: "Тільки укр." },
  { value: "pl", label: "Polski (PL)" },
  { value: "de", label: "Deutsch (DE)" },
  { value: "fr", label: "Français (FR)" },
  { value: "es", label: "Español (ES)" },
];

type RunState = "idle" | "running" | "success" | "failed" | "cancelled";

function App() {
  const [files, setFiles] = useState<AudioFileInfo[]>([]);
  const [probingCount, setProbingCount] = useState(0);
  const probing = probingCount > 0;
  const [probeError, setProbeError] = useState("");
  const [ffmpeg, setFfmpeg] = useState<FFmpegStatus>({ found: false });
  const [detectingFfmpeg, setDetectingFfmpeg] = useState(true);
  const [settings, setSettings] = useState(defaultSettings);
  const [settingsOpen, setSettingsOpen] = useState(false);
  const [savingSettings, setSavingSettings] = useState(false);
  const [operation, setOperation] = useState<
    "mix" | "concatenate" | "transcribe"
  >("mix");
  const [mixDuration, setMixDuration] = useState<MixDuration>("longest");
  const [concatMode, setConcatMode] = useState<ConcatMode>("auto");
  const [defaultLanguage, setDefaultLanguage] =
    useState<TranscriptionLanguage>("uk");
  const [transcriptFormat, setTranscriptFormat] =
    useState<TranscriptFormat>("txt");
  const [mergeOutputs, setMergeOutputs] = useState(false);
  const [format, setFormat] = useState<OutputFormat>("flac");
  const [flacCompression, setFlacCompression] = useState(8);
  const [wavCodec, setWavCodec] = useState<WavDepth>("pcm_s24le");
  const [sampleRate, setSampleRate] = useState<number | null>(null);
  const [channels, setChannels] = useState<number | null>(null);
  const [advancedOpen, setAdvancedOpen] = useState(false);
  const [outputName, setOutputName] = useState("combined");
  const [outputDirectory, setOutputDirectory] = useState("");
  const [runState, setRunState] = useState<RunState>("idle");
  const [startingJob, setStartingJob] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [progress, setProgress] = useState<ProcessingProgress | null>(null);
  const [liveLogs, setLiveLogs] = useState<string[]>([]);
  const [runningCommand, setRunningCommand] = useState<string[]>([]);
  const [result, setResult] = useState<ProcessingFinished | null>(null);
  const [failure, setFailure] = useState<ProcessingFailed | null>(null);
  const [overwriteJob, setOverwriteJob] = useState<ProcessingJob | null>(null);
  const [dragIndex, setDragIndex] = useState<number | null>(null);
  const [isDragOver, setIsDragOver] = useState(false);

  const filesRef = useRef(files);
  filesRef.current = files;
  const pendingPathsRef = useRef<Set<string>>(new Set());
  const openOutputFolderRef = useRef(settings.openOutputFolder);
  openOutputFolderRef.current = settings.openOutputFolder;
  const activeJobIdRef = useRef<string | null>(null);

  const refreshFfmpeg = useCallback(async () => {
    setDetectingFfmpeg(true);
    try {
      setFfmpeg(await invoke<FFmpegStatus>("detect_ffmpeg"));
    } finally {
      setDetectingFfmpeg(false);
    }
  }, []);

  useEffect(() => {
    void Promise.all([
      invoke<AppSettings>("load_settings").then(setSettings),
      refreshFfmpeg(),
    ]);
  }, [refreshFfmpeg]);

  const addPaths = useCallback(
    async (paths: string[]) => {
      const unique = dedupeIncomingPaths(
        paths,
        filesRef.current,
        pendingPathsRef.current,
      );
      if (!unique.length) return;
      for (const path of unique) {
        pendingPathsRef.current.add(path.toLocaleLowerCase());
      }
      setProbingCount((c) => c + 1);
      setProbeError("");
      try {
        const added = await invoke<AudioFileInfo[]>("probe_files", {
          paths: unique,
        });
        setFiles((current) =>
          mergeUniqueFiles(current, added, defaultLanguage),
        );
        if (!outputDirectory && added[0]) {
          const remembered = settings.rememberLastOutputFolder
            ? settings.lastOutputFolder
            : null;
          setOutputDirectory(remembered || parentDirectory(added[0].path));
        }
      } catch (error) {
        setProbeError(String(error));
      } finally {
        for (const path of unique) {
          pendingPathsRef.current.delete(path.toLocaleLowerCase());
        }
        setProbingCount((c) => Math.max(0, c - 1));
      }
    },
    [defaultLanguage, outputDirectory, settings],
  );

  const addPathsRef = useRef(addPaths);
  addPathsRef.current = addPaths;

  useEffect(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    const register = (promise: Promise<() => void>) => {
      void promise.then((unlisten) => {
        if (disposed) unlisten();
        else unlisteners.push(unlisten);
      });
    };

    register(
      getCurrentWebviewWindow().onDragDropEvent((event) => {
        if (event.payload.type === "enter" || event.payload.type === "over") {
          setIsDragOver(true);
        } else if (event.payload.type === "leave") {
          setIsDragOver(false);
        } else if (event.payload.type === "drop") {
          setIsDragOver(false);
          if (event.payload.paths.length > 0) {
            void addPathsRef.current(event.payload.paths);
          }
        }
      }),
    );
    register(
      listen<ProcessingProgress>("processing-progress", ({ payload }) => {
        if (activeJobIdRef.current && payload.jobId !== activeJobIdRef.current)
          return;
        setProgress(payload);
      }),
    );
    register(
      listen<ProcessingLog>("processing-log", ({ payload }) => {
        if (activeJobIdRef.current && payload.jobId !== activeJobIdRef.current)
          return;
        setLiveLogs((current) => {
          const next = [...current, payload.line];
          return next.length > 1000 ? next.slice(next.length - 1000) : next;
        });
      }),
    );
    register(
      listen<ProcessingFinished>("processing-finished", ({ payload }) => {
        if (activeJobIdRef.current && payload.jobId !== activeJobIdRef.current)
          return;
        activeJobIdRef.current = null;
        setCancelling(false);
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
        if (openOutputFolderRef.current)
          void openPath(parentDirectory(payload.outputPath));
      }),
    );
    register(
      listen<ProcessingFailed>("processing-failed", ({ payload }) => {
        if (activeJobIdRef.current && payload.jobId !== activeJobIdRef.current)
          return;
        activeJobIdRef.current = null;
        setCancelling(false);
        setRunState(payload.cancelled ? "cancelled" : "failed");
        setFailure(payload.cancelled ? null : payload);
      }),
    );
    register(
      listen("close-requested-during-processing", () => {
        if (
          window.confirm(
            "Audio processing is still running. Cancel it and close the application?",
          )
        ) {
          void invoke("request_app_exit");
        }
      }),
    );

    return () => {
      disposed = true;
      unlisteners.forEach((unlisten) => unlisten());
    };
  }, []);

  const chooseFiles = async () => {
    if (probing || runState === "running") return;
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
    transcription: {
      modelId: PARAKEET_MODEL_ID,
      defaultLanguage,
      format: transcriptFormat,
      mergeOutputs,
    },
    outputDirectory,
    outputName: outputName.trim(),
    replaceExisting,
  });

  const runJob = async (job: ProcessingJob) => {
    setOverwriteJob(null);
    activeJobIdRef.current = job.id;
    setCancelling(false);
    setRunState("running");
    setProgress({
      jobId: job.id,
      processedSeconds: 0,
      totalSeconds: totalDuration(files, operation, mixDuration),
      percent: 0,
      elapsedSeconds: 0,
    });
    setLiveLogs([]);
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
      activeJobIdRef.current = null;
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
    setStartingJob(true);
    try {
      const job = buildJob();
      if (await invoke<boolean>("destination_exists", { job })) {
        setOverwriteJob(job);
      } else {
        await runJob(job);
      }
    } catch (error) {
      setRunState("failed");
      setFailure({
        jobId: "startup",
        message: String(error),
        log: "",
        command: [],
        cancelled: false,
      });
    } finally {
      setStartingJob(false);
    }
  };

  const cancel = async () => {
    setCancelling(true);
    try {
      await invoke("cancel_processing");
    } catch (error) {
      setCancelling(false);
      setFailure({
        jobId: activeJobIdRef.current ?? "cancel",
        message: String(error),
        log: "",
        command: [],
        cancelled: false,
      });
    }
  };

  const valid =
    ffmpeg.found &&
    files.length > 0 &&
    outputName.trim().length > 0 &&
    outputDirectory.length > 0 &&
    !outputName.includes("/") &&
    !outputName.includes("\\");
  const command = result?.command ?? failure?.command ?? runningCommand;
  const activeTaskLabel = cancelling
    ? "Скасування…"
    : startingJob
      ? "Запуск завдання…"
      : probing
        ? "Читання файлів…"
        : savingSettings
          ? "Збереження налаштувань…"
          : detectingFfmpeg
            ? "Перевірка FFmpeg…"
            : runState === "running"
              ? operation === "transcribe"
                ? "Транскрипція…"
                : "Обробка аудіо…"
              : null;

  const updateTrack = (
    index: number,
    patch: Partial<Pick<AudioFileInfo, "speaker" | "language">>,
  ) => {
    setFiles((current) =>
      current.map((item, idx) =>
        idx === index ? { ...item, ...patch } : item,
      ),
    );
  };

  const applyLanguageToAll = () => {
    setFiles((current) =>
      current.map((item) => ({ ...item, language: defaultLanguage })),
    );
  };

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
          {activeTaskLabel && (
            <span className="status-pill busy" role="status" aria-live="polite">
              <LoaderCircle size={13} className="spin" />
              {activeTaskLabel}
            </span>
          )}
          <span
            className={`status-pill ${detectingFfmpeg ? "busy" : ffmpeg.found ? "ready" : "missing"}`}
          >
            <span />
            {detectingFfmpeg
              ? "Checking FFmpeg…"
              : ffmpeg.found
                ? "FFmpeg ready"
                : "FFmpeg not found"}
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
        {!detectingFfmpeg && !ffmpeg.found && (
          <div className="ffmpeg-warning">
            <AlertCircle />
            <div>
              <strong>FFmpeg was not found</strong>
              <span>
                The bundled copy is unavailable. Reinstall Audio Forge or select
                ffmpeg.exe manually.
              </span>
            </div>
            <button
              className="button secondary"
              onClick={refreshFfmpeg}
              disabled={detectingFfmpeg}
            >
              <RefreshCw size={16} className={detectingFfmpeg ? "spin" : ""} />
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

        <section
          className={`panel input-panel ${isDragOver ? "drag-over" : ""}`}
        >
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
                disabled={runState === "running" || probing}
                onClick={() => setFiles([])}
              >
                <Trash2 size={15} />
                Clear all
              </button>
            )}
          </div>
          {files.length === 0 ? (
            <div
              className={`drop-zone ${isDragOver ? "drag-over" : ""} ${probing ? "loading" : ""}`}
              onClick={chooseFiles}
            >
              <div className="drop-icon">
                {probing ? <LoaderCircle className="spin" /> : <FileAudio />}
              </div>
              <h3>
                {probing
                  ? "Читання аудіофайлів…"
                  : isDragOver
                    ? "Відпустіть файли для додавання"
                    : "Drop audio files here"}
              </h3>
              <p>
                {probing
                  ? "Визначаємо тривалість і параметри потоків через ffprobe"
                  : "FLAC, WAV, MP3, AAC, Opus and other FFmpeg formats"}
              </p>
              <button
                className="button primary"
                type="button"
                disabled={probing}
              >
                {probing ? (
                  <LoaderCircle size={17} className="spin" />
                ) : (
                  <Plus size={17} />
                )}
                {probing ? "Reading metadata…" : "Add files"}
              </button>
            </div>
          ) : (
            <>
              <div className="file-table-head">
                <span>Name</span>
                <span>Speaker</span>
                <span>Language</span>
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
                    onDragStart={(e) => {
                      if (
                        (e.target as HTMLElement).closest(
                          "input, select, button",
                        )
                      ) {
                        e.preventDefault();
                        return;
                      }
                      setDragIndex(index);
                    }}
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
                    <div className="speaker-cell">
                      <User size={13} className="cell-icon" />
                      <input
                        className="speaker-input"
                        value={file.speaker}
                        placeholder={`Мовець ${index + 1}`}
                        aria-label={`Speaker for ${file.name}`}
                        onChange={(e) =>
                          updateTrack(index, { speaker: e.target.value })
                        }
                        disabled={runState === "running"}
                      />
                    </div>
                    <select
                      className="lang-select"
                      value={file.language}
                      aria-label={`Language for ${file.name}`}
                      onChange={(e) =>
                        updateTrack(index, {
                          language: e.target.value as TranscriptionLanguage,
                        })
                      }
                      disabled={runState === "running"}
                    >
                      {LANGUAGE_OPTIONS.map((opt) => (
                        <option key={opt.value} value={opt.value}>
                          {opt.label}
                        </option>
                      ))}
                    </select>
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
              <button
                className={operation === "transcribe" ? "selected" : ""}
                onClick={() => setOperation("transcribe")}
                disabled={runState === "running"}
              >
                <span className="radio-dot" />
                <Mic />
                <div>
                  <strong>Transcribe (Parakeet)</strong>
                  <small>Per-track ASR merged by timestamps & speakers</small>
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
              ) : operation === "concatenate" ? (
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
              ) : (
                <>
                  <label>Мова файлів за замовчуванням</label>
                  <div className="default-lang-row">
                    <select
                      value={defaultLanguage}
                      onChange={(e) =>
                        setDefaultLanguage(
                          e.target.value as TranscriptionLanguage,
                        )
                      }
                      disabled={runState === "running"}
                    >
                      {LANGUAGE_OPTIONS.map((opt) => (
                        <option key={opt.value} value={opt.value}>
                          {opt.label}
                        </option>
                      ))}
                    </select>
                    <button
                      type="button"
                      className="button secondary compact-btn"
                      onClick={applyLanguageToAll}
                      disabled={files.length === 0 || runState === "running"}
                    >
                      <Languages size={14} />
                      Застосувати до всіх доріжок
                    </button>
                  </div>
                  <label className="merge-toggle">
                    <input
                      type="checkbox"
                      checked={mergeOutputs}
                      onChange={(e) => setMergeOutputs(e.target.checked)}
                      disabled={runState === "running"}
                    />
                    <span>
                      Об&apos;єднати всі доріжки у спільний файл за часовими
                      маркерами
                    </span>
                  </label>
                  <div className="model-badge">
                    <Mic size={14} />
                    <span>
                      Модель: <code>{PARAKEET_MODEL_ID}</code>
                    </span>
                  </div>
                  <div className="model-attribution">
                    <span>{PARAKEET_ATTRIBUTION}</span>
                  </div>
                  <small>
                    Кожна доріжка транскрибується окремо з позначенням мовця.
                    Спільний об&apos;єднаний файл створюється лише коли
                    увімкнено опцію вище.
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
                {operation === "transcribe" ? (
                  <select
                    value={transcriptFormat}
                    onChange={(e) =>
                      setTranscriptFormat(e.target.value as TranscriptFormat)
                    }
                    disabled={runState === "running"}
                  >
                    <option value="txt">TXT — Текст з таймкодами</option>
                    <option value="srt">SRT — Субтитри</option>
                    <option value="json">JSON — Структурований</option>
                  </select>
                ) : (
                  <select
                    value={format}
                    onChange={(e) => setFormat(e.target.value as OutputFormat)}
                    disabled={runState === "running"}
                  >
                    <option value="flac">FLAC</option>
                    <option value="wav">WAV</option>
                  </select>
                )}
              </div>
              <div className="field filename">
                <label>Filename</label>
                <div className="suffix-input">
                  <input
                    value={outputName}
                    onChange={(e) => setOutputName(e.target.value)}
                    disabled={runState === "running"}
                  />
                  <span>
                    .{operation === "transcribe" ? transcriptFormat : format}
                  </span>
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
            {operation === "transcribe" ? (
              <div className="transcribe-output-note">
                <FileText size={15} />
                <span>
                  {mergeOutputs ? (
                    <>
                      Буде збережено <strong>{files.length}</strong>{" "}
                      {files.length === 1
                        ? "окремий файл"
                        : "окремих файлів для кожної доріжки"}{" "}
                      та спільний об&apos;єднаний файл{" "}
                      <strong>
                        {outputName.trim() || "combined"}.{transcriptFormat}
                      </strong>
                      .
                    </>
                  ) : (
                    <>
                      За замовчуванням буде збережено{" "}
                      <strong>{files.length}</strong>{" "}
                      {files.length === 1
                        ? "окремий файл транскрипції"
                        : "окремих файлів транскрипції"}{" "}
                      (по одному на кожну доріжку/мовця з префіксом{" "}
                      <strong>{outputName.trim() || "combined"}</strong>).
                    </>
                  )}
                </span>
              </div>
            ) : format === "flac" ? (
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

        {operation !== "transcribe" && (
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
                      setChannels(
                        e.target.value ? Number(e.target.value) : null,
                      )
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
        )}

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
                    ? operation === "transcribe"
                      ? "Transcribing audio…"
                      : "Processing audio…"
                    : runState === "success"
                      ? operation === "transcribe"
                        ? "Transcription complete"
                        : "Processing complete"
                      : runState === "cancelled"
                        ? operation === "transcribe"
                          ? "Transcription cancelled"
                          : "Processing cancelled"
                        : operation === "transcribe"
                          ? "Transcription failed"
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
              <>
                {result.individualOutputs?.length ? (
                  !result.individualOutputs.includes(result.outputPath) && (
                    <p className="success-path" title={result.outputPath}>
                      Merged transcript saved to {result.outputPath}
                    </p>
                  )
                ) : (
                  <p className="success-path" title={result.outputPath}>
                    {result.usedStreamCopy ? "Stream copied to" : "Encoded to"}{" "}
                    {result.outputPath}
                  </p>
                )}
                {result.individualOutputs &&
                  result.individualOutputs.length > 0 && (
                    <div className="individual-outputs-list">
                      <strong>
                        Окремі файли доріжок ({result.individualOutputs.length}
                        ):
                      </strong>
                      <ul>
                        {result.individualOutputs.map((path) => (
                          <li key={path} title={path}>
                            {path}
                          </li>
                        ))}
                      </ul>
                    </div>
                  )}
                {result.transcriptPreview && (
                  <TranscriptPreview preview={result.transcriptPreview} />
                )}
              </>
            )}
            {failure && <ErrorDetails failure={failure} />}
            {(liveLogs.length > 0 ||
              (runState === "running" && operation === "transcribe")) && (
              <LiveLogPanel logs={liveLogs} running={runState === "running"} />
            )}
            {command.length > 0 && settings.showCommand && (
              <CommandDetails command={command} />
            )}
          </section>
        )}

        <div className="action-bar">
          <div>
            <span>
              {operation === "transcribe"
                ? "Total audio to transcribe"
                : "Estimated output length"}
            </span>
            <strong>
              {formatDuration(totalDuration(files, operation, mixDuration))}
            </strong>
          </div>
          {runState === "running" ? (
            <button
              className="button cancel"
              onClick={cancel}
              disabled={cancelling}
            >
              {cancelling ? <LoaderCircle className="spin" /> : <CircleStop />}
              {cancelling
                ? "Скасування…"
                : operation === "transcribe"
                  ? "Cancel transcription"
                  : "Cancel processing"}
            </button>
          ) : (
            <button
              className="button start"
              onClick={start}
              disabled={!valid || probing || startingJob}
            >
              {startingJob ? (
                <LoaderCircle className="spin" />
              ) : operation === "transcribe" ? (
                <Mic />
              ) : (
                <AudioLines />
              )}
              {startingJob
                ? "Запуск…"
                : operation === "transcribe"
                  ? "Start transcription"
                  : "Start processing"}
            </button>
          )}
        </div>
      </main>

      {settingsOpen && (
        <SettingsDialog
          settings={settings}
          ffmpeg={ffmpeg}
          saving={savingSettings}
          onClose={() => setSettingsOpen(false)}
          onSave={async (next, nextStatus) => {
            setSavingSettings(true);
            try {
              setSettings(next);
              setFfmpeg(nextStatus);
              await invoke("save_settings", { value: next });
              setSettingsOpen(false);
            } finally {
              setSavingSettings(false);
            }
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

function LiveLogPanel({ logs, running }: { logs: string[]; running: boolean }) {
  const boxRef = useRef<HTMLPreElement | null>(null);
  const [autoScroll, setAutoScroll] = useState(true);

  useEffect(() => {
    if (autoScroll && boxRef.current) {
      boxRef.current.scrollTop = boxRef.current.scrollHeight;
    }
  }, [logs, autoScroll]);

  const handleScroll = () => {
    const el = boxRef.current;
    if (!el) return;
    const nearBottom = el.scrollHeight - el.scrollTop - el.clientHeight <= 24;
    if (nearBottom !== autoScroll) {
      setAutoScroll(nearBottom);
    }
  };

  const content = logs.length
    ? logs.join("\n")
    : running
      ? "Очікування запуску воркера транскрибції…"
      : "";

  return (
    <div className="live-log-panel">
      <div className="live-log-header">
        <strong>Лог транскрибції в реальному часі</strong>
        <div className="live-log-controls">
          <label>
            <input
              type="checkbox"
              checked={autoScroll}
              onChange={(e) => setAutoScroll(e.target.checked)}
            />
            <span>Автопрокрутка</span>
          </label>
          {logs.length > 0 && (
            <button
              type="button"
              className="button secondary compact"
              onClick={() => navigator.clipboard.writeText(logs.join("\n"))}
            >
              Копіювати лог
            </button>
          )}
        </div>
      </div>
      <pre ref={boxRef} className="live-log-box" onScroll={handleScroll}>
        {content}
      </pre>
    </div>
  );
}

function TranscriptPreview({ preview }: { preview: string }) {
  const [openPreview, setOpenPreview] = useState(true);
  return (
    <div className="transcript-preview">
      <button
        className="text-button"
        onClick={() => setOpenPreview(!openPreview)}
      >
        <ChevronDown className={openPreview ? "open" : ""} />
        Попередній перегляд транскрипту
      </button>
      {openPreview && (
        <>
          <pre>{preview}</pre>
          <button
            className="button secondary compact"
            onClick={() => navigator.clipboard.writeText(preview)}
          >
            Copy transcript
          </button>
        </>
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
  saving,
  onClose,
  onSave,
}: {
  settings: AppSettings;
  ffmpeg: FFmpegStatus;
  saving: boolean;
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
              value={
                draft.ffmpegPath ??
                (status.source === "bundled"
                  ? "Bundled FFmpeg (automatic)"
                  : "Auto detected from PATH")
              }
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
                  ? status.source === "bundled"
                    ? "Bundled FFmpeg detected"
                    : "FFmpeg detected"
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
              Use bundled automatic detection
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
        <div className="settings-section">
          <label>AI Model &amp; Attribution</label>
          <div className="settings-attribution">
            <code>{PARAKEET_MODEL_ID}</code>
            <span>{PARAKEET_ATTRIBUTION}</span>
          </div>
        </div>
        <div className="modal-actions">
          <button className="button ghost" onClick={onClose} disabled={saving}>
            Cancel
          </button>
          <button
            className="button primary"
            disabled={!status.found || checking || saving}
            onClick={() =>
              onSave(
                draft.ffmpegPath && status.source !== "custom"
                  ? { ...draft, ffmpegPath: null }
                  : draft,
                status,
              )
            }
          >
            {saving && <LoaderCircle size={15} className="spin" />}
            {saving ? "Збереження…" : "Save settings"}
          </button>
        </div>
      </div>
    </div>
  );
}

export default App;
