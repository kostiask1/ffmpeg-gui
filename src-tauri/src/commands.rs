use crate::{ffmpeg, ffprobe, flac, models::*, progress, settings, transcribe};
use std::{
    fs,
    io::{BufRead, BufReader, Read},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    thread,
    time::Instant,
};
use tauri::{AppHandle, Emitter, Manager};

pub struct ActiveProcess {
    pub child: Arc<Mutex<Child>>,
    pub cancelled: Arc<AtomicBool>,
    pub partial_path: PathBuf,
    pub concat_list: Option<PathBuf>,
    pub extra_cleanup_paths: Vec<PathBuf>,
}

#[derive(Default)]
pub struct ProcessState(pub Mutex<Option<ActiveProcess>>);

fn cleanup_paths(paths: &[PathBuf]) {
    for path in paths {
        let _ = fs::remove_file(path);
    }
}

fn kill_process_tree(child: &mut Child) -> Result<(), String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let pid = child.id().to_string();
        let mut kill_cmd = Command::new("taskkill");
        kill_cmd
            .args(["/PID", &pid, "/T", "/F"])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .stdin(Stdio::null())
            .creation_flags(0x08000000);
        let _ = kill_cmd.status();
    }
    match child.try_wait() {
        Ok(Some(_)) => Ok(()),
        _ => child
            .kill()
            .map_err(|error| format!("Could not terminate process: {error}")),
    }
}

fn emit_live_log(app: &AppHandle, job_id: &str, stream: &str, line: &str) {
    let trimmed = line.trim_end_matches(&['\r', '\n'][..]);
    if trimmed.is_empty() {
        return;
    }
    let _ = app.emit(
        "processing-log",
        ProcessingLog {
            job_id: job_id.to_string(),
            line: trimmed.to_string(),
            stream: stream.to_string(),
        },
    );
}

fn should_skip_stderr_live_line(line: &str) -> bool {
    let trimmed = line.trim();
    trimmed.is_empty()
        || trimmed.starts_with("Loading weights:")
        || trimmed.contains("NOTE: Redirects are currently not supported in Windows or MacOs.")
}

fn move_final_output(
    partial_path: &Path,
    final_path: &Path,
    replace_existing: bool,
) -> Result<(), String> {
    if final_path.exists() {
        if !replace_existing {
            return Err("The destination file appeared while processing".into());
        }
        fs::remove_file(final_path)
            .map_err(|error| format!("Could not replace destination: {error}"))?;
    }
    fs::rename(partial_path, final_path)
        .map_err(|error| format!("Could not finalize output: {error}"))
}

fn spawn_stderr_reader(
    stderr: std::process::ChildStderr,
    live_emitter: Option<(AppHandle, String)>,
) -> (Arc<Mutex<String>>, thread::JoinHandle<()>) {
    let log = Arc::new(Mutex::new(String::new()));
    let log_writer = log.clone();
    let handle = thread::spawn(move || {
        let mut reader = BufReader::new(stderr);
        let mut buffer = [0_u8; 8192];
        let mut pending = String::new();
        while let Ok(count) = reader.read(&mut buffer) {
            if count == 0 {
                break;
            }
            let chunk = String::from_utf8_lossy(&buffer[..count]);
            if let Ok(mut target) = log_writer.lock() {
                target.push_str(&chunk);
            }
            if let Some((ref app, ref job_id)) = live_emitter {
                pending.push_str(&chunk);
                while let Some(newline_pos) = pending.find('\n') {
                    let raw_line = pending[..newline_pos].to_string();
                    pending.drain(..=newline_pos);
                    let cleaned = raw_line.rsplit('\r').next().unwrap_or("").trim();
                    if !should_skip_stderr_live_line(cleaned) {
                        emit_live_log(app, job_id, "stderr", cleaned);
                    }
                }
            }
        }
        if let Some((ref app, ref job_id)) = live_emitter {
            let cleaned = pending.rsplit('\r').next().unwrap_or("").trim();
            if !should_skip_stderr_live_line(cleaned) {
                emit_live_log(app, job_id, "stderr", cleaned);
            }
        }
    });
    (log, handle)
}

fn wait_for_child(child: &Arc<Mutex<Child>>) -> Option<std::process::ExitStatus> {
    loop {
        match child
            .lock()
            .ok()
            .and_then(|mut c| c.try_wait().ok())
            .flatten()
        {
            Some(status) => return Some(status),
            None => thread::sleep(std::time::Duration::from_millis(30)),
        }
    }
}

#[tauri::command]
pub fn load_settings(app: AppHandle) -> AppSettings {
    settings::load(&app)
}

#[tauri::command]
pub fn save_settings(app: AppHandle, value: AppSettings) -> Result<(), String> {
    settings::save(&app, &value)
}

fn detect_ffmpeg_sync(app: &AppHandle) -> FfmpegStatus {
    let saved = settings::load(app);
    ffmpeg::detect(saved.ffmpeg_path.as_deref())
}

#[tauri::command]
pub async fn detect_ffmpeg(app: AppHandle) -> Result<FfmpegStatus, String> {
    tauri::async_runtime::spawn_blocking(move || detect_ffmpeg_sync(&app))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn validate_ffmpeg_path(path: String) -> Result<FfmpegStatus, String> {
    tauri::async_runtime::spawn_blocking(move || ffmpeg::detect(Some(&path)))
        .await
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub async fn probe_files(app: AppHandle, paths: Vec<String>) -> Result<Vec<AudioFileInfo>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let status = detect_ffmpeg_sync(&app);
        let probe_path = status.ffprobe_path.ok_or_else(|| {
            status
                .error
                .unwrap_or_else(|| "ffprobe was not found".into())
        })?;
        paths
            .into_iter()
            .map(|path| ffprobe::probe(Path::new(&probe_path), Path::new(&path)))
            .collect()
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn destination_exists(job: ProcessingJob) -> bool {
    transcribe::destination_paths(&job)
        .map(|paths| paths.into_iter().any(|path| path.exists()))
        .unwrap_or(false)
}

fn start_transcription(
    app: AppHandle,
    state: &ProcessState,
    job: ProcessingJob,
    ffmpeg_executable: PathBuf,
) -> Result<ProcessingStarted, String> {
    let mut process_slot = state.0.lock().map_err(|_| "Process state is unavailable")?;
    if process_slot.is_some() {
        return Err("Another processing job is already running".into());
    }
    let built = transcribe::build(&ffmpeg_executable, &job, &std::env::temp_dir())?;
    let mut cleanup_list = vec![built.job_file_path.clone(), built.preview_path.clone()];
    if let Some(ref merged_partial) = built.merged_partial_path {
        cleanup_list.push(merged_partial.clone());
        let _ = fs::remove_file(merged_partial);
    }
    for item in &built.individual_outputs {
        cleanup_list.push(item.partial_path.clone());
        let _ = fs::remove_file(&item.partial_path);
    }
    cleanup_paths(&[built.preview_path.clone()]);

    let args = vec![
        "-u".into(),
        built.script_path.to_string_lossy().into_owned(),
        "--job-file".into(),
        built.job_file_path.to_string_lossy().into_owned(),
    ];
    let mut command = Command::new(&built.python_executable);
    command
        .args(&args)
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUTF8", "1")
        .env("PYTHONUNBUFFERED", "1")
        .env("HF_HUB_DISABLE_SYMLINKS_WARNING", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            cleanup_paths(&cleanup_list);
            return Err(format!(
                "Could not start Python transcription worker: {error}"
            ));
        }
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = kill_process_tree(&mut child);
        cleanup_paths(&cleanup_list);
        return Err("Could not capture transcription progress".into());
    };
    let Some(stderr) = child.stderr.take() else {
        let _ = kill_process_tree(&mut child);
        cleanup_paths(&cleanup_list);
        return Err("Could not capture transcription log".into());
    };

    let child = Arc::new(Mutex::new(child));
    let cancelled = Arc::new(AtomicBool::new(false));
    *process_slot = Some(ActiveProcess {
        child: child.clone(),
        cancelled: cancelled.clone(),
        partial_path: built
            .merged_partial_path
            .clone()
            .unwrap_or_else(|| built.job_file_path.clone()),
        concat_list: None,
        extra_cleanup_paths: cleanup_list.clone(),
    });
    drop(process_slot);

    let command_display = built.display_command.clone();
    let app_for_thread = app.clone();
    let response = ProcessingStarted {
        job_id: job.id.clone(),
        command: command_display.clone(),
        used_stream_copy: false,
    };

    emit_live_log(
        &app,
        &job.id,
        "stdout",
        &format!(
            "Запуск Python-воркера ({})",
            built.python_executable.to_string_lossy()
        ),
    );

    thread::spawn(move || {
        let (log, stderr_thread) =
            spawn_stderr_reader(stderr, Some((app_for_thread.clone(), job.id.clone())));
        let started = Instant::now();

        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some((processed, total)) = transcribe::parse_progress_line(&line) {
                let effective_total = if total > 0.0 {
                    total
                } else {
                    built.total_duration
                };
                let percent = if effective_total > 0.0 {
                    (processed / effective_total * 100.0).clamp(0.0, 100.0)
                } else {
                    0.0
                };
                let elapsed = started.elapsed().as_secs_f64();
                let speed = if elapsed > 0.0 && processed > 0.0 {
                    Some(processed / elapsed)
                } else {
                    None
                };
                let remaining = speed
                    .filter(|v| *v > 0.0)
                    .map(|v| ((effective_total - processed).max(0.0)) / v);
                let _ = app_for_thread.emit(
                    "processing-progress",
                    ProcessingProgress {
                        job_id: job.id.clone(),
                        processed_seconds: processed,
                        total_seconds: effective_total,
                        percent,
                        elapsed_seconds: elapsed,
                        remaining_seconds: remaining,
                        speed,
                    },
                );
            } else if let Some(msg) = transcribe::parse_log_line(&line) {
                if let Ok(mut target) = log.lock() {
                    target.push_str(msg);
                    target.push('\n');
                }
                emit_live_log(&app_for_thread, &job.id, "stdout", msg);
            } else if !line.trim().is_empty() {
                if let Ok(mut target) = log.lock() {
                    target.push_str(&line);
                    target.push('\n');
                }
                emit_live_log(&app_for_thread, &job.id, "stdout", &line);
            }
        }

        let exit = wait_for_child(&child);
        let _ = stderr_thread.join();
        let was_cancelled = cancelled.load(Ordering::SeqCst);
        let success = exit.is_some_and(|status| status.success()) && !was_cancelled;

        if success {
            let finalize_result = (|| -> Result<(String, Vec<String>, Option<String>), String> {
                let mut individual_paths = Vec::with_capacity(built.individual_outputs.len());
                for item in &built.individual_outputs {
                    move_final_output(&item.partial_path, &item.final_path, job.replace_existing)?;
                    individual_paths.push(item.final_path.to_string_lossy().into_owned());
                }
                let output_path = if let (Some(ref partial), Some(ref final_path)) =
                    (&built.merged_partial_path, &built.merged_final_path)
                {
                    move_final_output(partial, final_path, job.replace_existing)?;
                    final_path.to_string_lossy().into_owned()
                } else {
                    individual_paths
                        .first()
                        .cloned()
                        .unwrap_or_else(|| job.output_directory.clone())
                };
                let preview = fs::read_to_string(&built.preview_path)
                    .ok()
                    .map(|text| text.trim().to_string())
                    .filter(|text| !text.is_empty());
                Ok((output_path, individual_paths, preview))
            })();
            let _ = fs::remove_file(&built.job_file_path);
            let _ = fs::remove_file(&built.preview_path);

            match finalize_result {
                Ok((output_path, individual_outputs, transcript_preview)) => {
                    let _ = app_for_thread.emit(
                        "processing-finished",
                        ProcessingFinished {
                            job_id: job.id.clone(),
                            output_path,
                            command: command_display.clone(),
                            used_stream_copy: false,
                            individual_outputs,
                            transcript_preview,
                        },
                    );
                }
                Err(message) => {
                    cleanup_paths(&cleanup_list);
                    let _ = app_for_thread.emit(
                        "processing-failed",
                        ProcessingFailed {
                            job_id: job.id.clone(),
                            message,
                            log: log.lock().map(|v| v.clone()).unwrap_or_default(),
                            command: command_display.clone(),
                            cancelled: false,
                        },
                    );
                }
            }
        } else {
            cleanup_paths(&cleanup_list);
            let code = exit
                .and_then(|status| status.code())
                .map(|v| v.to_string())
                .unwrap_or_else(|| "unknown".into());
            let message = if was_cancelled {
                "Processing cancelled".into()
            } else {
                format!("Transcription exited with code {code}")
            };
            let _ = app_for_thread.emit(
                "processing-failed",
                ProcessingFailed {
                    job_id: job.id.clone(),
                    message,
                    log: log.lock().map(|v| v.clone()).unwrap_or_default(),
                    command: command_display,
                    cancelled: was_cancelled,
                },
            );
        }

        if let Some(state) = app_for_thread.try_state::<ProcessState>() {
            if let Ok(mut value) = state.0.lock() {
                *value = None;
            }
        }
    });

    Ok(response)
}

#[tauri::command]
pub async fn start_processing(
    app: AppHandle,
    job: ProcessingJob,
) -> Result<ProcessingStarted, String> {
    tauri::async_runtime::spawn_blocking(move || start_processing_sync(app, job))
        .await
        .map_err(|error| error.to_string())?
}

fn start_processing_sync(app: AppHandle, job: ProcessingJob) -> Result<ProcessingStarted, String> {
    if !Path::new(&job.output_directory).is_dir() {
        return Err("The output folder does not exist".into());
    }
    if destination_exists(job.clone()) && !job.replace_existing {
        return Err("The destination file already exists".into());
    }
    let status = detect_ffmpeg_sync(&app);
    let executable = PathBuf::from(status.ffmpeg_path.ok_or_else(|| {
        status
            .error
            .unwrap_or_else(|| "FFmpeg was not found".into())
    })?);

    let state = app.state::<ProcessState>();
    if job.operation == "transcribe" {
        return start_transcription(app.clone(), &state, job, executable);
    }

    let mut process_slot = state.0.lock().map_err(|_| "Process state is unavailable")?;
    if process_slot.is_some() {
        return Err("Another processing job is already running".into());
    }
    let built = ffmpeg::build(executable, &job, &std::env::temp_dir())?;
    let _ = fs::remove_file(&built.partial_path);

    let mut command = Command::new(&built.executable);
    command
        .args(&built.args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .stdin(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            if let Some(list) = &built.concat_list {
                let _ = fs::remove_file(list);
            }
            return Err(format!("Could not start FFmpeg: {error}"));
        }
    };
    let Some(stdout) = child.stdout.take() else {
        let _ = kill_process_tree(&mut child);
        let _ = fs::remove_file(&built.partial_path);
        if let Some(list) = &built.concat_list {
            let _ = fs::remove_file(list);
        }
        return Err("Could not capture FFmpeg progress".into());
    };
    let Some(stderr) = child.stderr.take() else {
        let _ = kill_process_tree(&mut child);
        let _ = fs::remove_file(&built.partial_path);
        if let Some(list) = &built.concat_list {
            let _ = fs::remove_file(list);
        }
        return Err("Could not capture FFmpeg log".into());
    };
    let child = Arc::new(Mutex::new(child));
    let cancelled = Arc::new(AtomicBool::new(false));
    *process_slot = Some(ActiveProcess {
        child: child.clone(),
        cancelled: cancelled.clone(),
        partial_path: built.partial_path.clone(),
        concat_list: built.concat_list.clone(),
        extra_cleanup_paths: Vec::new(),
    });
    drop(process_slot);

    let command_display = std::iter::once(built.executable.to_string_lossy().into_owned())
        .chain(built.args.clone())
        .collect::<Vec<_>>();
    let app_for_thread = app.clone();
    let response = ProcessingStarted {
        job_id: job.id.clone(),
        command: command_display.clone(),
        used_stream_copy: built.used_stream_copy,
    };
    thread::spawn(move || {
        let (log, stderr_thread) = spawn_stderr_reader(stderr, None);
        let started = Instant::now();
        let mut processed = 0.0;
        let mut speed = None;
        for line in BufReader::new(stdout).lines().map_while(Result::ok) {
            if let Some(value) = progress::parse_time(&line) {
                processed = value;
            }
            if let Some(value) = progress::parse_speed(&line) {
                speed = Some(value);
            }
            if line.starts_with("progress=") || line.starts_with("out_time_") {
                let percent = if built.total_duration > 0.0 {
                    (processed / built.total_duration * 100.0).clamp(0.0, 100.0)
                } else {
                    0.0
                };
                let remaining = speed
                    .filter(|v| *v > 0.0)
                    .map(|v| ((built.total_duration - processed).max(0.0)) / v);
                let _ = app_for_thread.emit(
                    "processing-progress",
                    ProcessingProgress {
                        job_id: job.id.clone(),
                        processed_seconds: processed,
                        total_seconds: built.total_duration,
                        percent,
                        elapsed_seconds: started.elapsed().as_secs_f64(),
                        remaining_seconds: remaining,
                        speed,
                    },
                );
            }
        }
        let exit = wait_for_child(&child);
        let _ = stderr_thread.join();
        let was_cancelled = cancelled.load(Ordering::SeqCst);
        let success = exit.is_some_and(|status| status.success()) && !was_cancelled;
        if let Some(list) = &built.concat_list {
            let _ = fs::remove_file(list);
        }
        if success {
            let move_result = (|| -> Result<(), String> {
                if built.used_stream_copy {
                    let input_paths = job
                        .inputs
                        .iter()
                        .map(|input| input.path.clone())
                        .collect::<Vec<_>>();
                    flac::repair_concatenated_stream_info(&built.partial_path, &input_paths)?;
                }
                move_final_output(&built.partial_path, &built.final_path, job.replace_existing)
            })();
            match move_result {
                Ok(()) => {
                    let _ = app_for_thread.emit(
                        "processing-finished",
                        ProcessingFinished {
                            job_id: job.id.clone(),
                            output_path: built.final_path.to_string_lossy().into(),
                            command: command_display.clone(),
                            used_stream_copy: built.used_stream_copy,
                            individual_outputs: Vec::new(),
                            transcript_preview: None,
                        },
                    );
                }
                Err(message) => {
                    let _ = fs::remove_file(&built.partial_path);
                    let _ = app_for_thread.emit(
                        "processing-failed",
                        ProcessingFailed {
                            job_id: job.id.clone(),
                            message,
                            log: log.lock().map(|v| v.clone()).unwrap_or_default(),
                            command: command_display.clone(),
                            cancelled: false,
                        },
                    );
                }
            }
        } else {
            let _ = fs::remove_file(&built.partial_path);
            let code = exit
                .and_then(|status| status.code())
                .map(|v| v.to_string())
                .unwrap_or_else(|| "unknown".into());
            let message = if was_cancelled {
                "Processing cancelled".into()
            } else {
                format!("FFmpeg exited with code {code}")
            };
            let _ = app_for_thread.emit(
                "processing-failed",
                ProcessingFailed {
                    job_id: job.id.clone(),
                    message,
                    log: log.lock().map(|v| v.clone()).unwrap_or_default(),
                    command: command_display,
                    cancelled: was_cancelled,
                },
            );
        }
        if let Some(state) = app_for_thread.try_state::<ProcessState>() {
            if let Ok(mut value) = state.0.lock() {
                *value = None;
            }
        }
    });
    Ok(response)
}

#[tauri::command]
pub async fn cancel_processing(app: AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let state = app.state::<ProcessState>();
        let child_arc = {
            let guard = state.0.lock().map_err(|_| "Process state is unavailable")?;
            let Some(active) = guard.as_ref() else {
                return Ok(());
            };
            active.cancelled.store(true, Ordering::SeqCst);
            active.child.clone()
        };
        let mut child = child_arc
            .lock()
            .map_err(|_| "Worker process is unavailable")?;
        kill_process_tree(&mut child)
    })
    .await
    .map_err(|error| error.to_string())?
}

#[tauri::command]
pub fn request_app_exit(app: AppHandle) {
    let state = app.state::<ProcessState>();
    if let Ok(guard) = state.0.lock() {
        if let Some(active) = guard.as_ref() {
            active.cancelled.store(true, Ordering::SeqCst);
            if let Ok(mut child) = active.child.lock() {
                let _ = kill_process_tree(&mut child);
                let _ = child.wait();
            }
            let _ = fs::remove_file(&active.partial_path);
            if let Some(list) = &active.concat_list {
                let _ = fs::remove_file(list);
            }
            cleanup_paths(&active.extra_cleanup_paths);
        }
    }
    app.exit(0);
}
