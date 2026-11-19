mod commands;
mod ffmpeg;
mod ffprobe;
mod flac;
mod models;
mod progress;
mod settings;
mod transcribe;

use commands::ProcessState;
use tauri::{Emitter, Manager, WindowEvent};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .manage(ProcessState::default())
        .invoke_handler(tauri::generate_handler![
            commands::load_settings,
            commands::save_settings,
            commands::detect_ffmpeg,
            commands::validate_ffmpeg_path,
            commands::probe_files,
            commands::destination_exists,
            commands::start_processing,
            commands::cancel_processing,
            commands::request_app_exit,
        ])
        .on_window_event(|window, event| {
            if let WindowEvent::CloseRequested { api, .. } = event {
                let running = window
                    .app_handle()
                    .state::<ProcessState>()
                    .0
                    .lock()
                    .map(|v| v.is_some())
                    .unwrap_or(false);
                if running {
                    api.prevent_close();
                    let _ = window.emit("close-requested-during-processing", ());
                }
            }
        })
        .run(tauri::generate_context!())
        .expect("error while running Audio Forge");
}
