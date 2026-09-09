# Audio Forge

Audio Forge is a Windows-first, fully local desktop application for mixing and concatenating audio with an external FFmpeg installation. It uses Tauri 2, React, TypeScript, Vite, and Rust. Audio data never enters JavaScript memory and is never uploaded, so multi-gigabyte files are processed with FFmpeg's normal streaming I/O.

## Features

- Detects `ffmpeg` and `ffprobe` from `PATH`, with a persistent custom `ffmpeg.exe` setting.
- Imports multiple files through a native dialog or OS drag and drop, probes metadata, and retains only their original paths.
- Reorders files with drag and drop or accessible up/down controls.
- Mixes all sources from time zero with `amix`, normalization, and longest/shortest/first duration policies.
- Concatenates in visible order with Auto, Stream Copy, and Re-encode modes.
- Auto stream copy is limited to compatible FLAC inputs and FLAC output; other jobs are encoded.
- Writes FLAC (compression 0–12) or WAV (16-bit PCM, 24-bit PCM, or 32-bit float).
- Supports optional output sample rate and mono/stereo conversion.
- Parses machine-readable FFmpeg progress, reports time/speed/ETA, and supports cancellation.
- Writes to `name.partial.ext`, then atomically renames after success. Partial outputs and concat lists are cleaned after failure or cancellation.
- Prompts before replacement, exposes full FFmpeg logs, and can copy the exact executable/argument list.

## Prerequisites

Install:

1. Node.js 20 or newer.
2. The Rust stable MSVC toolchain through [rustup](https://rustup.rs/).
3. Microsoft Visual Studio C++ Build Tools with the Windows SDK and **Desktop development with C++** workload.
4. FFmpeg containing both `ffmpeg.exe` and `ffprobe.exe`.

With Chocolatey, FFmpeg can be installed using:

```powershell
choco install ffmpeg
```

Restart the terminal after modifying `PATH`. Confirm the tools are available:

```powershell
ffmpeg -version
ffprobe -version
rustc --version
cargo --version
```

## Development

```powershell
npm install
npm run tauri dev
```

Frontend-only checks:

```powershell
npm run lint
npm test
npm run format:check
```

Rust checks:

```powershell
cargo fmt --manifest-path src-tauri/Cargo.toml --check
cargo test --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/core-tests/Cargo.toml
cargo test --manifest-path src-tauri/core-tests/Cargo.toml -- --ignored
```

## Windows build

Create the optimized frontend and Tauri installers with:

```powershell
npm run tauri build
```

Artifacts are written below `src-tauri\target\release\bundle`. Tauri may produce MSI and NSIS packages depending on the installed Windows tooling.

A verified x64 NSIS package from the current source is included at `artifacts\Audio Forge_0.1.0_x64-setup.exe`.

## Project structure

```text
src/
  App.tsx          React UI, native dialogs, app state, and Tauri event wiring
  types.ts         Typed frontend processing and metadata models
  utils.ts         Display and duration helpers
  styles.css       Offline, responsive single-window interface
src-tauri/src/
  commands.rs      Stable Tauri command/event boundary and process lifecycle
  ffmpeg.rs        Safe argument-array construction and concat-copy decisions
  ffprobe.rs       ffprobe JSON parsing and filesystem metadata
  flac.rs          STREAMINFO repair after lossless FLAC stream concatenation
  models.rs        Serializable backend models
  progress.rs      Machine-readable FFmpeg progress parsing
  settings.rs      Local JSON settings persistence
```

The frontend sends a typed `ProcessingJob`; it never constructs raw FFmpeg arguments. The operation field is an extensible string boundary so a future `multichannel` operation can receive its own builder without changing the UI/backend protocol shape.

## FFmpeg detection

At startup the backend loads application settings. When a custom executable path exists, that exact executable is validated with `-version`; otherwise `ffmpeg.exe` is resolved by Windows through `PATH`. The backend first looks for `ffprobe.exe` beside the resolved/custom FFmpeg executable and then falls back to `PATH`. Detection can be retried, or the user can browse to `ffmpeg.exe`. The selected location is saved under Tauri's per-user application configuration directory.

Every external process is started directly with an executable plus an argument array. User paths and names are never interpolated into a shell command, which preserves spaces, Ukrainian/Cyrillic text, quotes, and other Windows path characters.

## Auto concatenation decision

Auto mode chooses concat demuxer stream copy only when output is FLAC and every input audio stream is FLAC with the same sample rate, channel count, channel layout, and sample format. After copying, Audio Forge updates the output FLAC STREAMINFO sample count and clears the now-inapplicable per-file MD5 without decoding the audio. Otherwise it uses FFmpeg's concat filter and re-encodes with the chosen output settings. Explicit Stream Copy returns a validation error rather than silently copying incompatible inputs.

## Current limitations and TODOs

- The MVP targets Windows; other desktop platforms are not tested.
- Only FLAC and WAV outputs are exposed. MP3, AAC/M4A, and Opus can be added through the encoding model.
- Mix rows do not yet expose gain/mute controls. Per-track `volume` and `muted` values already exist in the processing model and FFmpeg filter builder, ready for row controls in a later UI update.
- No waveform, playback, trimming, crossfade, or multichannel routing is included.
- Input inspection is performed as one batch: if one selected file is invalid, that import reports the error and does not append the batch.
- Stream-copy safety is intentionally conservative; files outside the exact metadata match are re-encoded.
