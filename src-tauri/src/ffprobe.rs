use crate::models::AudioFileInfo;
use serde::Deserialize;
use std::{fs, path::Path, process::Command};

#[derive(Deserialize)]
struct ProbeResult {
    streams: Vec<ProbeStream>,
    format: ProbeFormat,
}

#[derive(Deserialize)]
struct ProbeStream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    sample_rate: Option<String>,
    channels: Option<u32>,
    channel_layout: Option<String>,
    sample_fmt: Option<String>,
    bits_per_raw_sample: Option<String>,
    bits_per_sample: Option<u32>,
    bit_rate: Option<String>,
    duration: Option<String>,
}

#[derive(Deserialize)]
struct ProbeFormat {
    duration: Option<String>,
    bit_rate: Option<String>,
    size: Option<String>,
}

pub fn probe(executable: &Path, path: &Path) -> Result<AudioFileInfo, String> {
    let output = Command::new(executable)
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .map_err(|error| format!("Could not start ffprobe: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "ffprobe failed for {}: {}",
            path.display(),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    parse_probe(path, &output.stdout)
}

fn parse_probe(path: &Path, json: &[u8]) -> Result<AudioFileInfo, String> {
    let parsed: ProbeResult = serde_json::from_slice(json)
        .map_err(|error| format!("Invalid ffprobe response: {error}"))?;
    let stream = parsed
        .streams
        .iter()
        .find(|stream| stream.codec_type.as_deref() == Some("audio"))
        .ok_or_else(|| format!("No audio stream found in {}", path.display()))?;
    let duration = stream
        .duration
        .as_deref()
        .or(parsed.format.duration.as_deref())
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    let metadata_size = fs::metadata(path).map(|m| m.len()).ok();
    Ok(AudioFileInfo {
        name: path
            .file_name()
            .map(|v| v.to_string_lossy().into())
            .unwrap_or_else(|| path.to_string_lossy().into()),
        path: path.to_string_lossy().into(),
        duration,
        codec: stream
            .codec_name
            .clone()
            .unwrap_or_else(|| "unknown".into()),
        sample_rate: stream.sample_rate.as_deref().and_then(|v| v.parse().ok()),
        channels: stream.channels,
        channel_layout: stream.channel_layout.clone(),
        sample_format: stream.sample_fmt.clone(),
        bit_depth: stream
            .bits_per_raw_sample
            .as_deref()
            .and_then(|v| v.parse().ok())
            .or(stream.bits_per_sample),
        bitrate: stream
            .bit_rate
            .as_deref()
            .or(parsed.format.bit_rate.as_deref())
            .and_then(|v| v.parse().ok()),
        size: metadata_size
            .or_else(|| parsed.format.size.as_deref().and_then(|v| v.parse().ok()))
            .unwrap_or(0),
        volume: 1.0,
        muted: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_audio_metadata_and_unicode_path() {
        let json = br#"{
          "streams": [{
            "codec_type": "audio", "codec_name": "flac", "sample_rate": "48000",
            "channels": 2, "channel_layout": "stereo", "sample_fmt": "s32",
            "bits_per_raw_sample": "24", "bit_rate": "900000", "duration": "12.5"
          }],
          "format": { "duration": "12.5", "bit_rate": "900000", "size": "123456" }
        }"#;
        let path = Path::new(r"D:\Музика\Сцена 01\доріжка №1.flac");
        let info = parse_probe(path, json).expect("metadata should parse");
        assert_eq!(info.name, "доріжка №1.flac");
        assert_eq!(info.codec, "flac");
        assert_eq!(info.sample_rate, Some(48_000));
        assert_eq!(info.channels, Some(2));
        assert_eq!(info.bit_depth, Some(24));
        assert_eq!(info.duration, 12.5);
        assert_eq!(info.size, 123_456);
    }

    #[test]
    fn rejects_files_without_audio_streams() {
        let json = br#"{"streams": [], "format": {"duration": "1"}}"#;
        let error = parse_probe(Path::new("video.mp4"), json).unwrap_err();
        assert!(error.contains("No audio stream"));
    }
}
