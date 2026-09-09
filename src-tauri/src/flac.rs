use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

const HEADER_SIZE: usize = 42;
const TOTAL_SAMPLES_MASK: u64 = (1_u64 << 36) - 1;

#[derive(Clone, Debug, PartialEq, Eq)]
struct StreamInfo {
    min_block_size: u16,
    max_block_size: u16,
    min_frame_size: u32,
    max_frame_size: u32,
    format_bits: u64,
    total_samples: u64,
}

fn read_stream_info(path: &Path) -> Result<([u8; HEADER_SIZE], StreamInfo), String> {
    let mut bytes = [0_u8; HEADER_SIZE];
    File::open(path)
        .and_then(|mut file| file.read_exact(&mut bytes))
        .map_err(|error| format!("Cannot read FLAC metadata from {}: {error}", path.display()))?;

    if &bytes[..4] != b"fLaC" || bytes[4] & 0x7f != 0 {
        return Err(format!(
            "{} does not begin with FLAC STREAMINFO",
            path.display()
        ));
    }
    let block_length = u32::from_be_bytes([0, bytes[5], bytes[6], bytes[7]]);
    if block_length != 34 {
        return Err(format!("{} has invalid FLAC STREAMINFO", path.display()));
    }

    let packed = u64::from_be_bytes(bytes[18..26].try_into().expect("fixed slice length"));
    Ok((
        bytes,
        StreamInfo {
            min_block_size: u16::from_be_bytes([bytes[8], bytes[9]]),
            max_block_size: u16::from_be_bytes([bytes[10], bytes[11]]),
            min_frame_size: u32::from_be_bytes([0, bytes[12], bytes[13], bytes[14]]),
            max_frame_size: u32::from_be_bytes([0, bytes[15], bytes[16], bytes[17]]),
            format_bits: packed & !TOTAL_SAMPLES_MASK,
            total_samples: packed & TOTAL_SAMPLES_MASK,
        },
    ))
}

fn merge_stream_info(values: &[StreamInfo]) -> Result<StreamInfo, String> {
    let first = values
        .first()
        .ok_or_else(|| "No FLAC inputs were provided".to_string())?;
    if values
        .iter()
        .any(|value| value.format_bits != first.format_bits)
    {
        return Err("FLAC STREAMINFO formats do not match".into());
    }

    let total_samples = if values.iter().any(|value| value.total_samples == 0) {
        0
    } else {
        values.iter().try_fold(0_u64, |total, value| {
            total
                .checked_add(value.total_samples)
                .filter(|sum| *sum <= TOTAL_SAMPLES_MASK)
                .ok_or_else(|| "Concatenated FLAC sample count is too large".to_string())
        })?
    };
    let known_min_frame_sizes = values.iter().all(|value| value.min_frame_size != 0);
    let known_max_frame_sizes = values.iter().all(|value| value.max_frame_size != 0);

    Ok(StreamInfo {
        min_block_size: values
            .iter()
            .map(|value| value.min_block_size)
            .min()
            .unwrap_or(first.min_block_size),
        max_block_size: values
            .iter()
            .map(|value| value.max_block_size)
            .max()
            .unwrap_or(first.max_block_size),
        min_frame_size: if known_min_frame_sizes {
            values
                .iter()
                .map(|value| value.min_frame_size)
                .min()
                .unwrap_or(first.min_frame_size)
        } else {
            0
        },
        max_frame_size: if known_max_frame_sizes {
            values
                .iter()
                .map(|value| value.max_frame_size)
                .max()
                .unwrap_or(first.max_frame_size)
        } else {
            0
        },
        format_bits: first.format_bits,
        total_samples,
    })
}

fn write_u24(target: &mut [u8], value: u32) {
    let bytes = value.to_be_bytes();
    target.copy_from_slice(&bytes[1..]);
}

/// FFmpeg's FLAC stream-copy muxer retains STREAMINFO from the first input.
/// Combine the input metadata so decoders can see every copied frame.
pub fn repair_concatenated_stream_info(output: &Path, inputs: &[String]) -> Result<(), String> {
    let input_paths: Vec<PathBuf> = inputs.iter().map(PathBuf::from).collect();
    let values = input_paths
        .iter()
        .map(|path| read_stream_info(path).map(|(_, info)| info))
        .collect::<Result<Vec<_>, _>>()?;
    let merged = merge_stream_info(&values)?;
    let (mut bytes, output_info) = read_stream_info(output)?;
    if output_info.format_bits != merged.format_bits {
        return Err("Output FLAC STREAMINFO does not match the inputs".into());
    }

    bytes[8..10].copy_from_slice(&merged.min_block_size.to_be_bytes());
    bytes[10..12].copy_from_slice(&merged.max_block_size.to_be_bytes());
    write_u24(&mut bytes[12..15], merged.min_frame_size);
    write_u24(&mut bytes[15..18], merged.max_frame_size);
    bytes[18..26].copy_from_slice(&(merged.format_bits | merged.total_samples).to_be_bytes());
    bytes[26..42].fill(0); // The original per-file MD5 no longer describes the joined stream.

    let mut file = OpenOptions::new()
        .write(true)
        .open(output)
        .map_err(|error| {
            format!(
                "Cannot update FLAC metadata in {}: {error}",
                output.display()
            )
        })?;
    file.seek(SeekFrom::Start(0))
        .and_then(|_| file.write_all(&bytes))
        .and_then(|_| file.flush())
        .map_err(|error| {
            format!(
                "Cannot update FLAC metadata in {}: {error}",
                output.display()
            )
        })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;

    fn info(samples: u64, min_frame: u32, max_frame: u32) -> StreamInfo {
        StreamInfo {
            min_block_size: 4096,
            max_block_size: 4096,
            min_frame_size: min_frame,
            max_frame_size: max_frame,
            format_bits: 0x0bb8_0200_0000_0000,
            total_samples: samples,
        }
    }

    #[test]
    fn merges_sample_counts_and_frame_bounds() {
        let merged = merge_stream_info(&[info(48_000, 100, 200), info(96_000, 80, 240)])
            .expect("metadata should merge");
        assert_eq!(merged.total_samples, 144_000);
        assert_eq!(merged.min_frame_size, 80);
        assert_eq!(merged.max_frame_size, 240);
    }

    #[test]
    fn keeps_unknown_counts_and_frame_sizes_unknown() {
        let merged = merge_stream_info(&[info(48_000, 100, 200), info(0, 0, 0)])
            .expect("metadata should merge");
        assert_eq!(merged.total_samples, 0);
        assert_eq!(merged.min_frame_size, 0);
        assert_eq!(merged.max_frame_size, 0);
    }

    #[test]
    fn rejects_different_audio_formats() {
        let first = info(48_000, 100, 200);
        let mut second = first.clone();
        second.format_bits += 1_u64 << 36;
        assert!(merge_stream_info(&[first, second]).is_err());
    }

    #[test]
    #[ignore = "requires ffmpeg and ffprobe on PATH"]
    fn repairs_real_stream_copy_concat_duration() {
        let directory = std::env::temp_dir().join(format!(
            "audio-forge-flac-test-{}-Музика",
            std::process::id()
        ));
        std::fs::create_dir_all(&directory).expect("temporary folder should be created");
        let first = directory.join("доріжка №1.flac");
        let second = directory.join("доріжка №2.flac");
        let output = directory.join("разом.flac");
        for (path, frequency) in [(&first, "440"), (&second, "880")] {
            let status = Command::new("ffmpeg")
                .args([
                    "-v",
                    "error",
                    "-y",
                    "-f",
                    "lavfi",
                    "-i",
                    &format!("sine=frequency={frequency}:duration=1"),
                    "-ar",
                    "48000",
                    "-ac",
                    "2",
                ])
                .arg(path)
                .status()
                .expect("ffmpeg should start");
            assert!(status.success());
        }
        let list = directory.join("список.txt");
        let list_contents = [&first, &second]
            .iter()
            .map(|path| format!("file '{}'\n", path.to_string_lossy().replace('\\', "/")))
            .collect::<String>();
        std::fs::write(&list, list_contents).expect("concat list should be written");
        let status = Command::new("ffmpeg")
            .args(["-v", "error", "-y", "-f", "concat", "-safe", "0", "-i"])
            .arg(&list)
            .args(["-c", "copy"])
            .arg(&output)
            .status()
            .expect("ffmpeg should concatenate");
        assert!(status.success());

        repair_concatenated_stream_info(
            &output,
            &[
                first.to_string_lossy().into_owned(),
                second.to_string_lossy().into_owned(),
            ],
        )
        .expect("STREAMINFO should be repaired");
        let (_, repaired) = read_stream_info(&output).expect("output metadata should be readable");
        assert_eq!(repaired.total_samples, 96_000);
        let probe = Command::new("ffprobe")
            .args([
                "-v",
                "error",
                "-show_entries",
                "format=duration",
                "-of",
                "default=noprint_wrappers=1:nokey=1",
            ])
            .arg(&output)
            .output()
            .expect("ffprobe should start");
        assert!(probe.status.success());
        let duration: f64 = String::from_utf8_lossy(&probe.stdout)
            .trim()
            .parse()
            .expect("duration should be numeric");
        assert!((duration - 2.0).abs() < 0.01, "duration was {duration}");
        let decoded = Command::new("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&output)
            .args(["-f", "s16le", "-acodec", "pcm_s16le", "pipe:1"])
            .output()
            .expect("ffmpeg decoder should start");
        assert!(decoded.status.success());
        assert_eq!(decoded.stdout.len(), 96_000 * 2 * 2);
        let _ = std::fs::remove_dir_all(directory);
    }
}
