use crate::models::ProcessingJob;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const EMBEDDED_WORKER_SCRIPT: &str = include_str!("../../scripts/transcribe_worker.py");
const DEFAULT_MODEL_ID: &str = "ihnatov-yuri/parakeet-tdt-0.6b-v3-uk";

#[derive(Clone, Debug)]
pub struct IndividualTrackOutput {
    pub partial_path: PathBuf,
    pub final_path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct BuiltTranscription {
    pub python_executable: PathBuf,
    pub script_path: PathBuf,
    pub job_file_path: PathBuf,
    pub merged_partial_path: Option<PathBuf>,
    pub merged_final_path: Option<PathBuf>,
    pub preview_path: PathBuf,
    pub individual_outputs: Vec<IndividualTrackOutput>,
    pub total_duration: f64,
    pub display_command: Vec<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkerTrackInput {
    index: usize,
    name: String,
    path: String,
    duration: f64,
    speaker: String,
    language: String,
    individual_partial_path: String,
    individual_final_path: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkerJobSpec {
    job_id: String,
    ffmpeg_path: String,
    model_id: String,
    default_language: String,
    format: String,
    merge_outputs: bool,
    output_directory: String,
    output_name: String,
    merged_partial_path: Option<String>,
    preview_path: String,
    inputs: Vec<WorkerTrackInput>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct WorkerProgressPayload {
    processed_seconds: f64,
    total_seconds: f64,
}

pub fn sanitize_filename_part(value: &str) -> String {
    let mut sanitized = String::with_capacity(value.len());
    let mut prev_underscore = false;
    for ch in value.trim().chars() {
        let invalid = matches!(ch, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
            || ch.is_control()
            || ch.is_whitespace();
        if invalid {
            if !prev_underscore && !sanitized.is_empty() {
                sanitized.push('_');
                prev_underscore = true;
            }
        } else {
            sanitized.push(ch);
            prev_underscore = false;
        }
    }
    let trimmed = sanitized.trim_matches(|c| c == '_' || c == '.');
    if trimmed.is_empty() {
        "track".into()
    } else {
        trimmed.to_string()
    }
}

pub fn output_extension(job: &ProcessingJob) -> Result<&'static str, String> {
    if job.operation == "transcribe" {
        match job.transcription.format.as_str() {
            "txt" => Ok("txt"),
            "srt" => Ok("srt"),
            "json" => Ok("json"),
            other => Err(format!("Unsupported transcript format: {other}")),
        }
    } else {
        match job.encoding.format.as_str() {
            "wav" => Ok("wav"),
            "flac" => Ok("flac"),
            other => Err(format!("Unsupported output format: {other}")),
        }
    }
}

pub fn build_individual_filename(
    output_name: &str,
    index: usize,
    speaker: &str,
    file_name: &str,
    ext: &str,
    partial: bool,
) -> String {
    let base = sanitize_filename_part(output_name);
    let speaker_slug = sanitize_filename_part(speaker);
    let file_stem = Path::new(file_name)
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| file_name.to_string());
    let stem_slug = sanitize_filename_part(&file_stem);
    let ordinal = format!("{:02}", index + 1);
    if partial {
        format!("{base}_{ordinal}_{speaker_slug}_{stem_slug}.partial.{ext}")
    } else {
        format!("{base}_{ordinal}_{speaker_slug}_{stem_slug}.{ext}")
    }
}

fn resolved_track_speaker(raw_speaker: &str, index: usize) -> String {
    let trimmed = raw_speaker.trim();
    if trimmed.is_empty() {
        format!("Мовець {}", index + 1)
    } else {
        trimmed.to_string()
    }
}

pub fn destination_paths(job: &ProcessingJob) -> Result<Vec<PathBuf>, String> {
    let trimmed_name = job.output_name.trim();
    if trimmed_name.is_empty() || trimmed_name.contains(['/', '\\']) {
        return Err("Output filename is invalid".into());
    }
    let ext = output_extension(job)?;
    let out_dir = Path::new(&job.output_directory);
    if job.operation == "transcribe" {
        let mut paths = Vec::new();
        if job.transcription.merge_outputs {
            paths.push(out_dir.join(format!("{trimmed_name}.{ext}")));
        }
        for (index, input) in job.inputs.iter().enumerate() {
            let speaker = resolved_track_speaker(&input.speaker, index);
            paths.push(out_dir.join(build_individual_filename(
                trimmed_name,
                index,
                &speaker,
                &input.name,
                ext,
                false,
            )));
        }
        Ok(paths)
    } else {
        Ok(vec![out_dir.join(format!("{trimmed_name}.{ext}"))])
    }
}

fn is_valid_python(candidate: &Path) -> bool {
    let mut cmd = Command::new(candidate);
    cmd.arg("--version").stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    cmd.output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

fn has_transcription_deps(candidate: &Path) -> bool {
    let mut cmd = Command::new(candidate);
    cmd.args(["-c", "import torch, transformers, soundfile"])
        .stdin(std::process::Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000);
    }
    cmd.output()
        .map(|out| out.status.success())
        .unwrap_or(false)
}

pub fn resolve_python() -> Result<PathBuf, String> {
    let mut candidates = Vec::new();
    if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
        let python_root = PathBuf::from(local_app_data)
            .join("Programs")
            .join("Python");
        candidates.push(python_root.join("Python314").join("python.exe"));
        if let Ok(entries) = fs::read_dir(&python_root) {
            for entry in entries.flatten() {
                let exe = entry.path().join("python.exe");
                if !candidates.contains(&exe) {
                    candidates.push(exe);
                }
            }
        }
    }
    let path_python = PathBuf::from(if cfg!(windows) {
        "python.exe"
    } else {
        "python3"
    });
    if !candidates.contains(&path_python) {
        candidates.push(path_python);
    }

    let valid_candidates: Vec<PathBuf> = candidates
        .into_iter()
        .filter(|candidate| {
            (candidate.components().count() == 1 || candidate.is_file())
                && is_valid_python(candidate)
        })
        .collect();

    for candidate in &valid_candidates {
        if has_transcription_deps(candidate) {
            return Ok(candidate.clone());
        }
    }
    if let Some(first_valid) = valid_candidates.into_iter().next() {
        return Ok(first_valid);
    }
    Err("Python 3 with PyTorch and transformers was not found on this machine".into())
}

pub fn resolve_worker_script(temp_dir: &Path) -> Result<PathBuf, String> {
    let mut candidates = vec![
        PathBuf::from("scripts").join("transcribe_worker.py"),
        PathBuf::from("..")
            .join("scripts")
            .join("transcribe_worker.py"),
    ];
    if let Ok(exe) = std::env::current_exe() {
        if let Some(exe_dir) = exe.parent() {
            candidates.push(exe_dir.join("scripts").join("transcribe_worker.py"));
        }
    }
    for candidate in candidates {
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    let extracted = temp_dir.join("audio-forge-transcribe-worker.py");
    fs::write(&extracted, EMBEDDED_WORKER_SCRIPT)
        .map_err(|error| format!("Cannot prepare transcription worker script: {error}"))?;
    Ok(extracted)
}

pub fn parse_progress_line(line: &str) -> Option<(f64, f64)> {
    let json_str = line.trim().strip_prefix("PROGRESS ")?;
    let payload: WorkerProgressPayload = serde_json::from_str(json_str).ok()?;
    Some((payload.processed_seconds, payload.total_seconds))
}

pub fn parse_log_line(line: &str) -> Option<&str> {
    line.trim_end_matches(&['\r', '\n'][..])
        .strip_prefix("LOG ")
}

pub fn build(
    ffmpeg_executable: &Path,
    job: &ProcessingJob,
    temp_dir: &Path,
) -> Result<BuiltTranscription, String> {
    let python_executable = resolve_python()?;
    let script_path = resolve_worker_script(temp_dir)?;
    build_with_paths(
        ffmpeg_executable,
        python_executable,
        script_path,
        job,
        temp_dir,
    )
}

pub fn build_with_paths(
    ffmpeg_executable: &Path,
    python_executable: PathBuf,
    script_path: PathBuf,
    job: &ProcessingJob,
    temp_dir: &Path,
) -> Result<BuiltTranscription, String> {
    if job.inputs.is_empty() {
        return Err("Add at least one input file".into());
    }
    let trimmed_name = job.output_name.trim();
    if trimmed_name.is_empty() || trimmed_name.contains(['/', '\\']) {
        return Err("Output filename is invalid".into());
    }
    if job.operation != "transcribe" {
        return Err("Unsupported transcription operation".into());
    }
    let ext = output_extension(job)?;
    let out_dir = Path::new(&job.output_directory);

    let merge_outputs = job.transcription.merge_outputs;
    let merged_final_path = merge_outputs.then(|| out_dir.join(format!("{trimmed_name}.{ext}")));
    let merged_partial_path =
        merge_outputs.then(|| out_dir.join(format!("{trimmed_name}.partial.{ext}")));
    let preview_path = temp_dir.join(format!("audio-forge-preview-{}.txt", job.id));
    let job_file_path = temp_dir.join(format!("audio-forge-transcribe-{}.json", job.id));

    let default_language = if job.transcription.default_language.trim().is_empty() {
        "uk".to_string()
    } else {
        job.transcription.default_language.trim().to_string()
    };
    let model_id = if job.transcription.model_id.trim().is_empty() {
        DEFAULT_MODEL_ID.to_string()
    } else {
        job.transcription.model_id.trim().to_string()
    };

    let mut worker_inputs = Vec::with_capacity(job.inputs.len());
    let mut individual_outputs = Vec::with_capacity(job.inputs.len());
    let mut total_duration = 0.0;

    for (index, input) in job.inputs.iter().enumerate() {
        let speaker = resolved_track_speaker(&input.speaker, index);
        let language = if input.language.trim().is_empty() {
            default_language.clone()
        } else {
            input.language.trim().to_string()
        };
        let individual_final = out_dir.join(build_individual_filename(
            trimmed_name,
            index,
            &speaker,
            &input.name,
            ext,
            false,
        ));
        let individual_partial = out_dir.join(build_individual_filename(
            trimmed_name,
            index,
            &speaker,
            &input.name,
            ext,
            true,
        ));
        total_duration += input.duration.max(0.0);
        worker_inputs.push(WorkerTrackInput {
            index,
            name: input.name.clone(),
            path: input.path.clone(),
            duration: input.duration,
            speaker,
            language,
            individual_partial_path: individual_partial.to_string_lossy().into_owned(),
            individual_final_path: individual_final.to_string_lossy().into_owned(),
        });
        individual_outputs.push(IndividualTrackOutput {
            partial_path: individual_partial,
            final_path: individual_final,
        });
    }

    let spec = WorkerJobSpec {
        job_id: job.id.clone(),
        ffmpeg_path: ffmpeg_executable.to_string_lossy().into_owned(),
        model_id,
        default_language,
        format: ext.to_string(),
        merge_outputs,
        output_directory: job.output_directory.clone(),
        output_name: trimmed_name.to_string(),
        merged_partial_path: merged_partial_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned()),
        preview_path: preview_path.to_string_lossy().into_owned(),
        inputs: worker_inputs,
    };

    let json = serde_json::to_string_pretty(&spec)
        .map_err(|error| format!("Cannot serialize transcription job: {error}"))?;
    fs::write(&job_file_path, json)
        .map_err(|error| format!("Cannot write transcription job file: {error}"))?;

    let display_command = vec![
        python_executable.to_string_lossy().into_owned(),
        "-u".into(),
        script_path.to_string_lossy().into_owned(),
        "--job-file".into(),
        job_file_path.to_string_lossy().into_owned(),
    ];

    Ok(BuiltTranscription {
        python_executable,
        script_path,
        job_file_path,
        merged_partial_path,
        merged_final_path,
        preview_path,
        individual_outputs,
        total_duration,
        display_command,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        AudioFileInfo, ConcatSettings, EncodingSettings, MixSettings, TranscriptionSettings,
    };

    fn sample_input(name: &str, speaker: &str, language: &str) -> AudioFileInfo {
        AudioFileInfo {
            name: name.into(),
            path: format!("D:\\Audio\\{name}"),
            duration: 15.0,
            codec: "flac".into(),
            sample_rate: Some(48000),
            channels: Some(2),
            channel_layout: Some("stereo".into()),
            sample_format: Some("s24".into()),
            bit_depth: Some(24),
            bitrate: None,
            size: 1024,
            volume: 1.0,
            muted: false,
            speaker: speaker.into(),
            language: language.into(),
        }
    }

    fn sample_job(inputs: Vec<AudioFileInfo>) -> ProcessingJob {
        ProcessingJob {
            id: "test-job-1".into(),
            inputs,
            operation: "transcribe".into(),
            mix: MixSettings {
                duration: "longest".into(),
            },
            concat: ConcatSettings {
                mode: "auto".into(),
            },
            encoding: EncodingSettings {
                format: "flac".into(),
                flac_compression: 8,
                wav_codec: "pcm_s24le".into(),
                sample_rate: None,
                channels: None,
            },
            transcription: TranscriptionSettings::default(),
            output_directory: "C:\\TempOut".into(),
            output_name: "interview".into(),
            replace_existing: false,
        }
    }

    #[test]
    fn sanitizes_windows_filename_parts_preserving_cyrillic() {
        assert_eq!(
            sanitize_filename_part("  Олена: гість/1? "),
            "Олена_гість_1"
        );
        assert_eq!(sanitize_filename_part("...   ..."), "track");
    }

    #[test]
    fn builds_individual_filenames_with_ukrainian_speakers() {
        assert_eq!(
            build_individual_filename("діалог", 0, "Олена Коваль", "доріжка №1.flac", "txt", false),
            "діалог_01_Олена_Коваль_доріжка_№1.txt"
        );
        assert_eq!(
            build_individual_filename("діалог", 1, "Speaker 2", "track2.wav", "srt", true),
            "діалог_02_Speaker_2_track2.partial.srt"
        );
    }

    #[test]
    fn parses_worker_progress_line() {
        assert_eq!(
            parse_progress_line(r#"PROGRESS {"processedSeconds": 12.5, "totalSeconds": 40.0}"#),
            Some((12.5, 40.0))
        );
        assert_eq!(parse_progress_line("Loading model..."), None);
        assert_eq!(
            parse_log_line("LOG [12:00:00] Модель завантажено\r\n"),
            Some("[12:00:00] Модель завантажено")
        );
        assert_eq!(parse_log_line("PROGRESS {}"), None);
    }

    #[test]
    fn builds_transcription_job_spec_and_outputs() {
        let temp_dir = std::env::temp_dir();
        let mut job = sample_job(vec![
            sample_input("mic1.flac", "Олена", "uk"),
            sample_input("mic2.wav", "", "en"),
        ]);
        job.transcription.format = "srt".into();

        // Default: merge_outputs is false -> only per-speaker individual files
        let dest_without_merge =
            destination_paths(&job).expect("destination paths should build without merge");
        assert_eq!(dest_without_merge.len(), 2);
        assert!(dest_without_merge[0].ends_with("interview_01_Олена_mic1.srt"));
        assert!(dest_without_merge[1].ends_with("interview_02_Мовець_2_mic2.srt"));

        let built_no_merge = build_with_paths(
            Path::new("ffmpeg.exe"),
            PathBuf::from("python.exe"),
            PathBuf::from("transcribe_worker.py"),
            &job,
            &temp_dir,
        )
        .expect("transcription build without merge should succeed");
        assert!(built_no_merge.merged_final_path.is_none());
        assert!(built_no_merge.merged_partial_path.is_none());
        assert_eq!(built_no_merge.individual_outputs.len(), 2);
        let raw_no_merge =
            fs::read_to_string(&built_no_merge.job_file_path).expect("job JSON must exist");
        assert!(raw_no_merge.contains("\"mergeOutputs\": false"));
        let _ = fs::remove_file(&built_no_merge.job_file_path);

        // Enabled: merge_outputs is true -> both individual files and merged file
        job.transcription.merge_outputs = true;
        let dest_with_merge =
            destination_paths(&job).expect("destination paths should build with merge");
        assert_eq!(dest_with_merge.len(), 3);
        assert!(dest_with_merge[0].ends_with("interview.srt"));

        let built = build_with_paths(
            Path::new("ffmpeg.exe"),
            PathBuf::from("python.exe"),
            PathBuf::from("transcribe_worker.py"),
            &job,
            &temp_dir,
        )
        .expect("transcription build with merge should succeed");

        assert!(built
            .merged_final_path
            .as_ref()
            .is_some_and(|p| p.ends_with("interview.srt")));
        assert!(built
            .merged_partial_path
            .as_ref()
            .is_some_and(|p| p.ends_with("interview.partial.srt")));
        assert_eq!(built.individual_outputs.len(), 2);
        assert!(built.individual_outputs[0]
            .final_path
            .ends_with("interview_01_Олена_mic1.srt"));
        assert!(built.individual_outputs[1]
            .final_path
            .ends_with("interview_02_Мовець_2_mic2.srt"));
        assert_eq!(built.total_duration, 30.0);

        let raw_json = fs::read_to_string(&built.job_file_path).expect("job JSON must exist");
        assert!(raw_json.contains("\"mergeOutputs\": true"));
        assert!(raw_json.contains("\"speaker\": \"Олена\""));
        assert!(raw_json.contains("\"speaker\": \"Мовець 2\""));
        assert!(raw_json.contains("\"language\": \"en\""));
        let _ = fs::remove_file(&built.job_file_path);
    }

    #[test]
    fn rejects_empty_inputs_and_bad_format() {
        let temp_dir = std::env::temp_dir();
        let empty = sample_job(vec![]);
        assert!(build_with_paths(
            Path::new("ffmpeg.exe"),
            PathBuf::from("python.exe"),
            PathBuf::from("transcribe_worker.py"),
            &empty,
            &temp_dir,
        )
        .is_err());

        let mut bad_fmt = sample_job(vec![sample_input("a.wav", "Мовець 1", "uk")]);
        bad_fmt.transcription.format = "docx".into();
        assert!(build_with_paths(
            Path::new("ffmpeg.exe"),
            PathBuf::from("python.exe"),
            PathBuf::from("transcribe_worker.py"),
            &bad_fmt,
            &temp_dir,
        )
        .is_err());
    }
}
