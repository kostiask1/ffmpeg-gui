use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AudioFileInfo {
    pub name: String,
    pub path: String,
    pub duration: f64,
    pub codec: String,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
    pub channel_layout: Option<String>,
    pub sample_format: Option<String>,
    pub bit_depth: Option<u32>,
    pub bitrate: Option<u64>,
    pub size: u64,
    #[serde(default = "default_track_volume")]
    pub volume: f64,
    #[serde(default)]
    pub muted: bool,
    #[serde(default)]
    pub speaker: String,
    #[serde(default = "default_track_language")]
    pub language: String,
}

fn default_track_volume() -> f64 {
    1.0
}

fn default_track_language() -> String {
    "uk".into()
}

fn default_model_id() -> String {
    "ihnatov-yuri/parakeet-tdt-0.6b-v3-uk".into()
}

fn default_transcript_format() -> String {
    "txt".into()
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppSettings {
    pub ffmpeg_path: Option<String>,
    pub open_output_folder: bool,
    pub show_command: bool,
    pub remember_last_output_folder: bool,
    pub last_output_folder: Option<String>,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            ffmpeg_path: None,
            open_output_folder: false,
            show_command: false,
            remember_last_output_folder: true,
            last_output_folder: None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FfmpegStatus {
    pub found: bool,
    pub source: Option<String>,
    pub ffmpeg_path: Option<String>,
    pub ffprobe_path: Option<String>,
    pub version: Option<String>,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingJob {
    pub id: String,
    pub inputs: Vec<AudioFileInfo>,
    pub operation: String,
    pub mix: MixSettings,
    pub concat: ConcatSettings,
    pub encoding: EncodingSettings,
    #[serde(default)]
    pub transcription: TranscriptionSettings,
    pub output_directory: String,
    pub output_name: String,
    pub replace_existing: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TranscriptionSettings {
    #[serde(default = "default_model_id")]
    pub model_id: String,
    #[serde(default = "default_track_language")]
    pub default_language: String,
    #[serde(default = "default_transcript_format")]
    pub format: String,
    #[serde(default)]
    pub merge_outputs: bool,
}

impl Default for TranscriptionSettings {
    fn default() -> Self {
        Self {
            model_id: default_model_id(),
            default_language: default_track_language(),
            format: default_transcript_format(),
            merge_outputs: false,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct MixSettings {
    pub duration: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ConcatSettings {
    pub mode: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EncodingSettings {
    pub format: String,
    pub flac_compression: u8,
    pub wav_codec: String,
    pub sample_rate: Option<u32>,
    pub channels: Option<u32>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingProgress {
    pub job_id: String,
    pub processed_seconds: f64,
    pub total_seconds: f64,
    pub percent: f64,
    pub elapsed_seconds: f64,
    pub remaining_seconds: Option<f64>,
    pub speed: Option<f64>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingStarted {
    pub job_id: String,
    pub command: Vec<String>,
    pub used_stream_copy: bool,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingLog {
    pub job_id: String,
    pub line: String,
    pub stream: String,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingFinished {
    pub job_id: String,
    pub output_path: String,
    pub command: Vec<String>,
    pub used_stream_copy: bool,
    #[serde(default)]
    pub individual_outputs: Vec<String>,
    #[serde(default)]
    pub transcript_preview: Option<String>,
}

#[derive(Clone, Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessingFailed {
    pub job_id: String,
    pub message: String,
    pub log: String,
    pub command: Vec<String>,
    pub cancelled: bool,
}
