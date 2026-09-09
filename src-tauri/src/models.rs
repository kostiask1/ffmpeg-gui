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
}

fn default_track_volume() -> f64 {
    1.0
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
    pub output_directory: String,
    pub output_name: String,
    pub replace_existing: bool,
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
pub struct ProcessingFinished {
    pub job_id: String,
    pub output_path: String,
    pub command: Vec<String>,
    pub used_stream_copy: bool,
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
