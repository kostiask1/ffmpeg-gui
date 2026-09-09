use crate::{ffmpeg, ffprobe, flac, models::*, progress, settings};
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
use tauri::{AppHandle, Emitter, Manager, State};

pub struct ActiveProcess {
    pub child: Arc<Mutex<Child>>,
    pub cancelled: Arc<AtomicBool>,
    pub partial_path: PathBuf,
    pub concat_list: Option<PathBuf>,
}

#[derive(Default)]
pub struct ProcessState(pub Mutex<Option<ActiveProcess>>);

#[tauri::command]
pub fn load_settings(app: AppHandle) -> AppSettings {
    settings::load(&app)
}

#[tauri::command]
pub fn save_settings(app: AppHandle, value: AppSettings) -> Result<(), String> {
    settings::save(&app, &value)
}

#[tauri::command]
pub fn detect_ffmpeg(app: AppHandle) -> FfmpegStatus {
    let saved = settings::load(&app);
    ffmpeg::detect(saved.ffmpeg_path.as_deref())
}

#[tauri::command]
pub fn validate_ffmpeg_path(path: String) -> FfmpegStatus {
    ffmpeg::detect(Some(&path))
}

#[tauri::command]
pub fn probe_files(app: AppHandle, paths: Vec<String>) -> Result<Vec<AudioFileInfo>, String> {
    let status = detect_ffmpeg(app);
    let probe_path = status.ffprobe_path.ok_or_else(|| {
        status
            .error
            .unwrap_or_else(|| "ffprobe was not found".into())
    })?;
    paths
        .into_iter()
        .map(|path| ffprobe::probe(Path::new(&probe_path), Path::new(&path)))
        .collect()
}

#[tauri::command]
pub fn destination_exists(job: ProcessingJob) -> bool {
    let extension = if job.encoding.format == "wav" {
        "wav"
    } else {
        "flac"
    };
    Path::new(&job.output_directory)
        .join(format!("{}.{}", job.output_name.trim(), extension))
        .exists()
}

#[tauri::command]
pub fn start_processing(
    app: AppHandle,
    state: State<'_, ProcessState>,
    job: ProcessingJob,
) -> Result<ProcessingStarted, String> {
    let mut process_slot = state.0.lock().map_err(|_| "Process state is unavailable")?;
    if process_slot.is_some() {
        return Err("Another processing job is already running".into());
    }
    if !Path::new(&job.output_directory).is_dir() {
        return Err("The output folder does not exist".into());
    }
    if destination_exists(job.clone()) && !job.replace_existing {
        return Err("The destination file already exists".into());
    }
    let status = detect_ffmpeg(app.clone());
    let executable = PathBuf::from(status.ffmpeg_path.ok_or_else(|| {
        status
            .error
            .unwrap_or_else(|| "FFmpeg was not found".into())
    })?);
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
        let _ = child.kill();
        let _ = fs::remove_file(&built.partial_path);
        if let Some(list) = &built.concat_list {
            let _ = fs::remove_file(list);
        }
        return Err("Could not capture FFmpeg progress".into());
    };
    let Some(stderr) = child.stderr.take() else {
        let _ = child.kill();
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
        let log = Arc::new(Mutex::new(String::new()));
        let log_writer = log.clone();
        let stderr_thread = thread::spawn(move || {
            let mut reader = BufReader::new(stderr);
            let mut buffer = [0_u8; 8192];
            while let Ok(count) = reader.read(&mut buffer) {
                if count == 0 {
                    break;
                }
                if let Ok(mut target) = log_writer.lock() {
                    target.push_str(&String::from_utf8_lossy(&buffer[..count]));
                }
            }
        });
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
        let exit = loop {
            match child
                .lock()
                .ok()
                .and_then(|mut c| c.try_wait().ok())
                .flatten()
            {
                Some(status) => break Some(status),
                None => thread::sleep(std::time::Duration::from_millis(30)),
            }
        };
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
                if built.final_path.exists() {
                    if !job.replace_existing {
                        return Err("The destination file appeared while processing".into());
                    }
                    fs::remove_file(&built.final_path)
                        .map_err(|error| format!("Could not replace destination: {error}"))?;
                }
                fs::rename(&built.partial_path, &built.final_path)
                    .map_err(|error| format!("Could not finalize output: {error}"))
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
pub fn cancel_processing(state: State<'_, ProcessState>) -> Result<(), String> {
    let guard = state.0.lock().map_err(|_| "Process state is unavailable")?;
    if let Some(active) = guard.as_ref() {
        active.cancelled.store(true, Ordering::SeqCst);
        active
            .child
            .lock()
            .map_err(|_| "FFmpeg process is unavailable")?
            .kill()
            .map_err(|error| format!("Could not terminate FFmpeg: {error}"))?;
    }
    Ok(())
}

#[tauri::command]
pub fn request_app_exit(app: AppHandle, state: State<'_, ProcessState>) {
    if let Ok(guard) = state.0.lock() {
        if let Some(active) = guard.as_ref() {
            active.cancelled.store(true, Ordering::SeqCst);
            if let Ok(mut child) = active.child.lock() {
                let _ = child.kill();
                let _ = child.wait();
            }
            let _ = fs::remove_file(&active.partial_path);
            if let Some(list) = &active.concat_list {
                let _ = fs::remove_file(list);
            }
        }
    }
    app.exit(0);
}
