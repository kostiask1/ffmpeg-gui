use crate::models::{AudioFileInfo, FfmpegStatus, ProcessingJob};
use std::{
    path::{Path, PathBuf},
    process::Command,
};

pub struct BuiltCommand {
    pub executable: PathBuf,
    pub args: Vec<String>,
    pub final_path: PathBuf,
    pub partial_path: PathBuf,
    pub concat_list: Option<PathBuf>,
    pub used_stream_copy: bool,
    pub total_duration: f64,
}

fn validate(executable: &Path) -> Option<String> {
    let output = Command::new(executable).arg("-version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .next()
        .map(str::to_owned)
}

fn companion_probe(ffmpeg: &Path) -> PathBuf {
    let filename = if cfg!(windows) {
        "ffprobe.exe"
    } else {
        "ffprobe"
    };
    ffmpeg
        .parent()
        .map(|p| p.join(filename))
        .filter(|p| p.exists())
        .unwrap_or_else(|| PathBuf::from(filename))
}

fn inspect(executable: PathBuf, source: &str) -> Option<FfmpegStatus> {
    let version = validate(&executable)?;
    let probe = companion_probe(&executable);
    validate(&probe)?;
    Some(FfmpegStatus {
        found: true,
        source: Some(source.into()),
        ffmpeg_path: Some(executable.to_string_lossy().into()),
        ffprobe_path: Some(probe.to_string_lossy().into()),
        version: Some(version),
        error: None,
    })
}

fn bundled_executable() -> Option<PathBuf> {
    let executable = std::env::current_exe().ok()?;
    let executable_directory = executable.parent()?;
    let sidecar_directory = if executable_directory.ends_with("deps") {
        executable_directory.parent()?
    } else {
        executable_directory
    };
    let candidate = sidecar_directory.join(if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    });
    candidate.exists().then_some(candidate)
}

pub fn detect(custom: Option<&str>) -> FfmpegStatus {
    if let Some(custom_path) = custom {
        if let Some(status) = inspect(PathBuf::from(custom_path), "custom") {
            return status;
        }
    }
    if let Some(executable) = bundled_executable() {
        if let Some(status) = inspect(executable, "bundled") {
            return status;
        }
    }
    let path_executable = PathBuf::from(if cfg!(windows) {
        "ffmpeg.exe"
    } else {
        "ffmpeg"
    });
    inspect(path_executable, "path").unwrap_or(FfmpegStatus {
        found: false,
        source: None,
        ffmpeg_path: None,
        ffprobe_path: None,
        version: None,
        error: Some("FFmpeg was not found".into()),
    })
}

pub fn stream_copy_safe(inputs: &[AudioFileInfo], job: &ProcessingJob) -> bool {
    if inputs.is_empty() || job.encoding.format != "flac" {
        return false;
    }
    let first = &inputs[0];
    first.codec.eq_ignore_ascii_case("flac")
        && inputs.iter().all(|item| {
            item.codec.eq_ignore_ascii_case("flac")
                && item.sample_rate == first.sample_rate
                && item.channels == first.channels
                && item.channel_layout == first.channel_layout
                && item.sample_format == first.sample_format
        })
}

fn concat_line(path: &str) -> String {
    let portable = path.replace('\\', "/");
    format!("file '{}'\n", portable.replace('\'', "'\\''"))
}

pub fn build(
    executable: PathBuf,
    job: &ProcessingJob,
    temp_dir: &Path,
) -> Result<BuiltCommand, String> {
    if job.inputs.is_empty() {
        return Err("Add at least one input file".into());
    }
    if job.output_name.trim().is_empty() || job.output_name.contains(['/', '\\']) {
        return Err("Output filename is invalid".into());
    }
    if job.operation != "mix" && job.operation != "concatenate" {
        return Err("This operation is not implemented yet".into());
    }
    let extension = match job.encoding.format.as_str() {
        "flac" => "flac",
        "wav" => "wav",
        _ => return Err("Unsupported output format".into()),
    };
    let final_path =
        Path::new(&job.output_directory).join(format!("{}.{}", job.output_name.trim(), extension));
    let partial_path = Path::new(&job.output_directory).join(format!(
        "{}.partial.{}",
        job.output_name.trim(),
        extension
    ));
    let total_duration = if job.operation == "concatenate" {
        job.inputs.iter().map(|i| i.duration).sum()
    } else {
        match job.mix.duration.as_str() {
            "shortest" => job
                .inputs
                .iter()
                .map(|i| i.duration)
                .fold(f64::INFINITY, f64::min),
            "first" => job.inputs[0].duration,
            _ => job.inputs.iter().map(|i| i.duration).fold(0.0, f64::max),
        }
    };
    let safe = stream_copy_safe(&job.inputs, job);
    let used_stream_copy = job.operation == "concatenate"
        && match job.concat.mode.as_str() {
            "copy" => {
                if !safe {
                    return Err(
                        "Stream Copy requires compatible FLAC inputs and FLAC output".into(),
                    );
                }
                true
            }
            "auto" => safe,
            _ => false,
        };
    let mut args = vec![
        "-hide_banner".into(),
        "-y".into(),
        "-progress".into(),
        "pipe:1".into(),
        "-nostats".into(),
    ];
    let mut concat_list = None;
    if job.operation == "concatenate" && used_stream_copy {
        let list = temp_dir.join(format!("audio-forge-{}.txt", job.id));
        let content: String = job
            .inputs
            .iter()
            .map(|input| concat_line(&input.path))
            .collect();
        std::fs::write(&list, content)
            .map_err(|error| format!("Cannot create temporary concat list: {error}"))?;
        args.extend([
            "-f".into(),
            "concat".into(),
            "-safe".into(),
            "0".into(),
            "-i".into(),
            list.to_string_lossy().into(),
        ]);
        concat_list = Some(list);
    } else if job.operation == "mix" {
        for input in &job.inputs {
            args.extend(["-i".into(), input.path.clone()]);
        }
        let mut filter = String::new();
        let mut labels = String::new();
        for (index, input) in job.inputs.iter().enumerate() {
            let volume = if input.muted {
                0.0
            } else {
                input.volume.clamp(0.0, 10.0)
            };
            filter.push_str(&format!("[{index}:a]volume={volume}[track{index}];"));
            labels.push_str(&format!("[track{index}]"));
        }
        filter.push_str(&format!(
            "{labels}amix=inputs={}:duration={}:normalize=1[mixed]",
            job.inputs.len(),
            job.mix.duration
        ));
        args.extend([
            "-filter_complex".into(),
            filter,
            "-map".into(),
            "[mixed]".into(),
        ]);
    } else {
        for input in &job.inputs {
            args.extend(["-i".into(), input.path.clone()]);
        }
        let target_rate = job
            .encoding
            .sample_rate
            .or(job.inputs[0].sample_rate)
            .unwrap_or(48_000);
        let target_channels = job
            .encoding
            .channels
            .or(job.inputs[0].channels)
            .unwrap_or(2);
        let layout = if target_channels == 1 {
            "mono"
        } else {
            "stereo"
        };
        let mut filter = String::new();
        let mut labels = String::new();
        for index in 0..job.inputs.len() {
            filter.push_str(&format!(
                "[{index}:a]aresample={target_rate}:async=1:first_pts=0,aformat=sample_rates={target_rate}:channel_layouts={layout}[a{index}];"
            ));
            labels.push_str(&format!("[a{index}]"));
        }
        filter.push_str(&format!(
            "{labels}concat=n={}:v=0:a=1[concatenated]",
            job.inputs.len()
        ));
        args.extend([
            "-filter_complex".into(),
            filter,
            "-map".into(),
            "[concatenated]".into(),
        ]);
    }
    if used_stream_copy {
        args.extend(["-c".into(), "copy".into()]);
    } else {
        match job.encoding.format.as_str() {
            "flac" => args.extend([
                "-c:a".into(),
                "flac".into(),
                "-compression_level".into(),
                job.encoding.flac_compression.min(12).to_string(),
            ]),
            "wav" => args.extend(["-c:a".into(), job.encoding.wav_codec.clone()]),
            _ => unreachable!(),
        }
        if let Some(rate) = job.encoding.sample_rate {
            args.extend(["-ar".into(), rate.to_string()]);
        }
        if let Some(channels) = job.encoding.channels {
            args.extend(["-ac".into(), channels.to_string()]);
        }
    }
    args.push(partial_path.to_string_lossy().into());
    Ok(BuiltCommand {
        executable,
        args,
        final_path,
        partial_path,
        concat_list,
        used_stream_copy,
        total_duration,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn input(path: &str, rate: u32) -> AudioFileInfo {
        AudioFileInfo {
            name: path.into(),
            path: path.into(),
            duration: 10.0,
            codec: "flac".into(),
            sample_rate: Some(rate),
            channels: Some(2),
            channel_layout: Some("stereo".into()),
            sample_format: Some("s32".into()),
            bit_depth: Some(24),
            bitrate: None,
            size: 1,
            volume: 1.0,
            muted: false,
        }
    }

    #[test]
    #[ignore = "requires ffmpeg and ffprobe on PATH"]
    fn detects_external_tools_from_path() {
        let status = detect(None);
        assert!(status.found, "{}", status.error.unwrap_or_default());
        assert!(status.ffprobe_path.is_some());
    }
    fn job(inputs: Vec<AudioFileInfo>) -> ProcessingJob {
        ProcessingJob {
            id: "test".into(),
            inputs,
            operation: "concatenate".into(),
            mix: crate::models::MixSettings {
                duration: "longest".into(),
            },
            concat: crate::models::ConcatSettings {
                mode: "auto".into(),
            },
            encoding: crate::models::EncodingSettings {
                format: "flac".into(),
                flac_compression: 8,
                wav_codec: "pcm_s24le".into(),
                sample_rate: None,
                channels: None,
            },
            output_directory: "C:\\out".into(),
            output_name: "result".into(),
            replace_existing: false,
        }
    }
    #[test]
    fn compatible_flac_can_copy() {
        assert!(stream_copy_safe(
            &[input("a.flac", 48000), input("b.flac", 48000)],
            &job(vec![input("a", 48000)])
        ));
    }
    #[test]
    fn differing_rates_reencode() {
        let inputs = vec![input("a.flac", 44100), input("b.flac", 48000)];
        assert!(!stream_copy_safe(&inputs, &job(inputs.clone())));
    }
    #[test]
    fn concat_list_preserves_unicode_and_spaces() {
        assert_eq!(
            concat_line("D:\\Музика\\Сцена 01\\доріжка №1.flac"),
            "file 'D:/Музика/Сцена 01/доріжка №1.flac'\n"
        );
    }

    #[test]
    fn mix_uses_amix_and_partial_output() {
        let mut value = job(vec![
            input("D:\\Музика\\one.flac", 48_000),
            input("two.flac", 48_000),
        ]);
        value.operation = "mix".into();
        value.mix.duration = "shortest".into();
        value.inputs[0].volume = 0.5;
        value.inputs[1].muted = true;
        let temp = std::env::temp_dir();
        let built =
            build(PathBuf::from("ffmpeg.exe"), &value, &temp).expect("command should build");
        let filter = built
            .args
            .windows(2)
            .find(|pair| pair[0] == "-filter_complex")
            .map(|pair| &pair[1])
            .unwrap();
        assert!(filter.contains("amix=inputs=2:duration=shortest:normalize=1"));
        assert!(filter.contains("[0:a]volume=0.5[track0]"));
        assert!(filter.contains("[1:a]volume=0[track1]"));
        assert!(built.partial_path.ends_with("result.partial.flac"));
        assert_eq!(built.total_duration, 10.0);
    }

    #[test]
    fn incompatible_auto_concat_uses_normalizing_filter() {
        let inputs = vec![input("first.flac", 44_100), input("second.flac", 48_000)];
        let value = job(inputs);
        let temp = std::env::temp_dir();
        let built =
            build(PathBuf::from("ffmpeg.exe"), &value, &temp).expect("command should build");
        assert!(!built.used_stream_copy);
        assert!(built.concat_list.is_none());
        assert!(built
            .args
            .iter()
            .any(|arg| arg.contains("concat=n=2:v=0:a=1")));
        assert_eq!(built.total_duration, 20.0);
    }

    #[test]
    fn explicit_copy_rejects_incompatible_inputs() {
        let inputs = vec![input("first.flac", 44_100), input("second.flac", 48_000)];
        let mut value = job(inputs);
        value.concat.mode = "copy".into();
        let error = build(PathBuf::from("ffmpeg.exe"), &value, &std::env::temp_dir())
            .err()
            .expect("copy must fail");
        assert!(error.contains("compatible FLAC"));
    }
}
