from __future__ import annotations

import argparse
from datetime import datetime
import json
import os
import subprocess
import sys
import time
from pathlib import Path
from typing import Any

os.environ.setdefault("HF_HUB_DISABLE_SYMLINKS_WARNING", "1")

SAMPLE_RATE = 16000
BYTES_PER_SAMPLE = 4
MAX_CHUNK_SEC = 30.0
TARGET_CHUNK_SEC = 28.0
MIN_CUT_SEC = 22.0
PUNCT_ONLY = set(".,?!:;…—-")
SENTENCE_END_PUNCT = (".", "?", "!", "…")

np: Any = None
torch: Any = None
DecodeStream: Any = None
AutoModelForTDT: Any = None
AutoProcessor: Any = None


def configure_stdio() -> None:
    if hasattr(sys.stdout, "reconfigure"):
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")
    if hasattr(sys.stderr, "reconfigure"):
        sys.stderr.reconfigure(encoding="utf-8", errors="replace")


def emit_log(message: str) -> None:
    stamp = datetime.now().strftime("%H:%M:%S")
    print(f"LOG [{stamp}] {message}", flush=True)


def emit_progress(processed_seconds: float, total_seconds: float) -> None:
    payload = {
        "processedSeconds": round(max(0.0, processed_seconds), 3),
        "totalSeconds": round(max(0.0, total_seconds), 3),
    }
    print(f"PROGRESS {json.dumps(payload, ensure_ascii=False)}", flush=True)


def ensure_ml_imports() -> None:
    global np, torch, DecodeStream, AutoModelForTDT, AutoProcessor
    if np is not None:
        return
    emit_log("Ініціалізація PyTorch та Transformers...")
    import numpy as _np
    import torch as _torch
    from tokenizers.decoders import DecodeStream as _DecodeStream
    from transformers import AutoModelForTDT as _AutoModelForTDT
    from transformers import AutoProcessor as _AutoProcessor

    np = _np
    torch = _torch
    DecodeStream = _DecodeStream
    AutoModelForTDT = _AutoModelForTDT
    AutoProcessor = _AutoProcessor


def _find_local_model_dir(model_id: str) -> Path | None:
    candidates: list[Path] = []
    local_app_data = os.environ.get("LOCALAPPDATA")
    if local_app_data:
        candidates.append(
            Path(local_app_data) / "Audio Forge" / "models" / "parakeet-tdt-0.6b-v3-uk"
        )

    hf_roots: list[Path] = []
    hf_home = os.environ.get("HF_HOME")
    if hf_home:
        hf_roots.append(Path(hf_home) / "hub")
    user_profile = os.environ.get("USERPROFILE") or str(Path.home())
    if user_profile:
        hf_roots.append(Path(user_profile) / ".cache" / "huggingface" / "hub")

    repo_slug = f"models--{model_id.replace('/', '--')}"
    for hub_root in hf_roots:
        snapshots_dir = hub_root / repo_slug / "snapshots"
        if snapshots_dir.is_dir():
            for child in snapshots_dir.iterdir():
                if child.is_dir():
                    candidates.append(child)

    for directory in candidates:
        if (directory / "config.json").is_file() and (directory / "model.safetensors").is_file():
            return directory
    return None


def load_model_and_processor(model_id: str):
    t0 = time.perf_counter()
    device = "cuda" if torch.cuda.is_available() else "cpu"
    use_bf16 = device == "cuda" and torch.cuda.is_bf16_supported()
    dtype = torch.bfloat16 if use_bf16 else torch.float32
    device_label = torch.cuda.get_device_name(0) if device == "cuda" else "CPU"
    emit_log(f"Завантаження моделі {model_id} на {device_label} ({dtype})...")

    try:
        processor = AutoProcessor.from_pretrained(model_id, local_files_only=True)
        model = AutoModelForTDT.from_pretrained(model_id, dtype=dtype, local_files_only=True)
    except Exception:
        local_dir = _find_local_model_dir(model_id)
        if local_dir is not None:
            processor = AutoProcessor.from_pretrained(str(local_dir), local_files_only=True)
            model = AutoModelForTDT.from_pretrained(
                str(local_dir), dtype=dtype, local_files_only=True
            )
        else:
            emit_log("Локальний кеш не знайдено — завантаження моделі з Hugging Face...")
            processor = AutoProcessor.from_pretrained(model_id, local_files_only=False)
            model = AutoModelForTDT.from_pretrained(model_id, dtype=dtype, local_files_only=False)

    model = model.to(device).eval()
    if hasattr(model, "decoder") and hasattr(model.decoder, "lstm"):
        model.decoder.lstm.flatten_parameters()

    elapsed = time.perf_counter() - t0
    emit_log(f"Модель готова до роботи ({elapsed:.1f} с).")
    return processor, model, device


def build_language_masks(processor, device: str) -> dict[str, Any]:
    blank_id = getattr(processor, "blank_token_id", 8192)
    vocab = processor.tokenizer.get_vocab()

    ru_only_ids: list[int] = []
    cyrillic_ids: list[int] = []
    latin_ids: list[int] = []

    for token_str, token_id in vocab.items():
        if token_id < 274 or token_id >= blank_id:
            continue
        if any(ch in "ыэъёЫЭЪЁ" for ch in token_str):
            ru_only_ids.append(token_id)
        if any("\u0400" <= ch <= "\u04FF" for ch in token_str):
            cyrillic_ids.append(token_id)
        if any("a" <= ch.lower() <= "z" for ch in token_str) and not any(
            ch.isdigit() for ch in token_str
        ):
            latin_ids.append(token_id)

    def to_tensor(ids: list[int]):
        if not ids:
            return None
        return torch.tensor(sorted(set(ids)), dtype=torch.long, device=device)

    uk_mask = to_tensor(ru_only_ids)
    uk_strict_mask = to_tensor(ru_only_ids + latin_ids)
    latin_lang_mask = to_tensor(cyrillic_ids)

    return {
        "uk": uk_mask,
        "auto": uk_mask,
        "uk_strict": uk_strict_mask,
        "en": latin_lang_mask,
        "pl": latin_lang_mask,
        "de": latin_lang_mask,
        "fr": latin_lang_mask,
        "es": latin_lang_mask,
    }


def _find_best_cut_sample(audio: Any) -> int:
    min_samples = int(MIN_CUT_SEC * SAMPLE_RATE)
    max_samples = int(MAX_CHUNK_SEC * SAMPLE_RATE)
    target_samples = int(TARGET_CHUNK_SEC * SAMPLE_RATE)
    win_samples = int(0.1 * SAMPLE_RATE)
    step_samples = int(0.05 * SAMPLE_RATE)

    total_samples = len(audio)
    search_start = min_samples
    search_end = min(total_samples - win_samples, max_samples)
    best_cut = min(total_samples, target_samples)

    if search_end > search_start:
        best_rms = float("inf")
        pos = search_start
        while pos + win_samples <= search_end:
            window = audio[pos : pos + win_samples]
            rms = float(np.mean(window * window))
            if rms < best_rms:
                best_rms = rms
                best_cut = pos + win_samples // 2
            pos += step_samples

    return best_cut if best_cut > 0 else min(total_samples, target_samples)


def stream_audio_chunks(ffmpeg_path: str, input_path: str):
    cmd = [
        ffmpeg_path,
        "-nostdin",
        "-v",
        "error",
        "-i",
        input_path,
        "-f",
        "f32le",
        "-acodec",
        "pcm_f32le",
        "-ac",
        "1",
        "-ar",
        str(SAMPLE_RATE),
        "pipe:1",
    ]
    creationflags = subprocess.CREATE_NO_WINDOW if sys.platform == "win32" else 0
    proc = subprocess.Popen(
        cmd,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        stdin=subprocess.DEVNULL,
        creationflags=creationflags,
    )

    max_samples = int(MAX_CHUNK_SEC * SAMPLE_RATE)
    max_bytes = max_samples * BYTES_PER_SAMPLE
    read_block = 4 * SAMPLE_RATE * BYTES_PER_SAMPLE  # 4 seconds per read
    buf = bytearray()
    offset_samples = 0

    try:
        assert proc.stdout is not None
        while True:
            data = proc.stdout.read(read_block)
            if data:
                buf.extend(data)
            while len(buf) >= max_bytes:
                audio_win = np.frombuffer(buf[:max_bytes], dtype=np.float32)
                cut_samples = _find_best_cut_sample(audio_win)
                cut_bytes = cut_samples * BYTES_PER_SAMPLE
                chunk_audio = np.frombuffer(buf[:cut_bytes], dtype=np.float32).copy()
                yield chunk_audio, offset_samples / float(SAMPLE_RATE)
                del buf[:cut_bytes]
                offset_samples += cut_samples
            if not data:
                break

        usable_samples = len(buf) // BYTES_PER_SAMPLE
        if usable_samples > 0:
            usable_bytes = usable_samples * BYTES_PER_SAMPLE
            chunk_audio = np.frombuffer(buf[:usable_bytes], dtype=np.float32).copy()
            yield chunk_audio, offset_samples / float(SAMPLE_RATE)

        proc.stdout.close()
        stderr_data = proc.stderr.read() if proc.stderr is not None else b""
        if proc.stderr is not None:
            proc.stderr.close()
        ret = proc.wait()
        if ret != 0:
            err = stderr_data.decode("utf-8", errors="replace").strip()
            raise RuntimeError(f"FFmpeg decode failed for {input_path}: {err or ret}")
    finally:
        if proc.poll() is None:
            proc.terminate()
            try:
                proc.wait(timeout=5)
            except subprocess.TimeoutExpired:
                proc.kill()


def transcribe_track_streaming(
    ffmpeg_path: str,
    input_path: str,
    speaker: str,
    track_dur_hint: float,
    processor,
    model,
    device: str,
    active_mask_holder: dict[str, Any],
    mask_tensor: Any,
    cumulative_before: float,
    total_seconds: float,
) -> tuple[list[dict], float]:
    active_mask_holder["ids"] = mask_tensor

    frame_rate = (
        processor.feature_extractor.hop_length
        / processor.feature_extractor.sampling_rate
        * 8
    )
    blank_id = getattr(processor, "blank_token_id", 8192)
    pad_id = processor.tokenizer.pad_token_id
    skip_ids = {x for x in (pad_id, blank_id) if x is not None}

    tokens: list[dict] = []
    processed_in_track = 0.0
    silent_skipped_sec = 0.0
    min_audio_samples = int(0.05 * SAMPLE_RATE)

    for chunk_audio, chunk_offset_sec in stream_audio_chunks(ffmpeg_path, input_path):
        chunk_dur = len(chunk_audio) / float(SAMPLE_RATE)
        rms = float(np.sqrt(np.mean(chunk_audio * chunk_audio))) if len(chunk_audio) else 0.0
        peak = float(np.max(np.abs(chunk_audio))) if len(chunk_audio) else 0.0

        if len(chunk_audio) < min_audio_samples or (rms < 1e-4 and peak < 1e-3):
            silent_skipped_sec += chunk_dur
            if silent_skipped_sec >= 300.0:
                emit_log(
                    f"[{format_timestamp_txt(chunk_offset_sec)}] {speaker}: пропуск тиші ({int(silent_skipped_sec)} с)..."
                )
                silent_skipped_sec = 0.0
        else:
            silent_skipped_sec = 0.0
            inputs = processor(chunk_audio, sampling_rate=SAMPLE_RATE, return_tensors="pt")
            inputs = {
                k: (
                    v.to(device=device, dtype=model.dtype)
                    if torch.is_floating_point(v)
                    else v.to(device=device)
                )
                for k, v in inputs.items()
            }
            max_new_tokens = max(256, int(chunk_dur * 30))
            with torch.inference_mode():
                outputs = model.generate(**inputs, max_new_tokens=max_new_tokens)

            seq = outputs.sequences[0].detach().cpu().tolist()
            durs = outputs.durations[0].detach().cpu().tolist()
            frame_pos = 0
            stream = DecodeStream(skip_special_tokens=True)
            chunk_tokens: list[dict] = []

            for idx, tid in enumerate(seq):
                dur_val = int(durs[idx]) if idx < len(durs) else 0
                frame_start = frame_pos
                frame_pos += dur_val

                if tid in skip_ids or tid < 234:
                    continue
                piece = stream.step(processor.tokenizer._tokenizer, int(tid))
                if piece is not None and piece != "":
                    t_start = chunk_offset_sec + frame_start * frame_rate
                    t_end = chunk_offset_sec + (frame_start + max(1, dur_val)) * frame_rate
                    chunk_tokens.append(
                        {
                            "token": piece,
                            "start": t_start,
                            "end": max(t_start, t_end),
                        }
                    )

            if chunk_tokens:
                tokens.extend(chunk_tokens)
                snippet = "".join(t["token"] for t in chunk_tokens).strip()
                if snippet:
                    if len(snippet) > 140:
                        snippet = snippet[:137] + "..."
                    emit_log(
                        f"[{format_timestamp_txt(chunk_tokens[0]['start'])} - {format_timestamp_txt(chunk_tokens[-1]['end'])}] {speaker}: {snippet}"
                    )

        processed_in_track += chunk_dur
        if track_dur_hint > 0.0:
            current_total_progress = cumulative_before + min(track_dur_hint, processed_in_track)
            emit_progress(min(total_seconds, current_total_progress), total_seconds)
        else:
            current_total_progress = cumulative_before + processed_in_track
            emit_progress(current_total_progress, max(total_seconds, current_total_progress))

    return tokens, processed_in_track


def tokens_to_segments(
    tokens: list[dict],
    speaker: str,
    language: str,
    file_name: str,
    track_index: int,
) -> list[dict]:
    if not tokens:
        return []

    words: list[dict] = []
    for tok in tokens:
        piece = tok["token"]
        if not piece:
            continue
        if piece in PUNCT_ONLY and words:
            words[-1]["word"] += piece
            words[-1]["end"] = max(words[-1]["end"], tok["end"])
            continue

        stripped = piece.lstrip(" ")
        if not stripped:
            continue

        if piece.startswith(" ") or not words:
            words.append(
                {
                    "word": stripped,
                    "start": tok["start"],
                    "end": tok["end"],
                }
            )
        else:
            words[-1]["word"] += piece
            words[-1]["end"] = max(words[-1]["end"], tok["end"])

    if not words:
        return []

    segments: list[dict] = []
    seg_words: list[dict] = [words[0]]

    for word in words[1:]:
        prev_word = seg_words[-1]
        gap = word["start"] - prev_word["end"]
        seg_dur = word["end"] - seg_words[0]["start"]
        prev_seg_dur = prev_word["end"] - seg_words[0]["start"]
        ends_sentence = prev_word["word"].rstrip().endswith(SENTENCE_END_PUNCT)

        should_split = (
            gap >= 0.85
            or (ends_sentence and (prev_seg_dur >= 1.0 or gap >= 0.2))
            or seg_dur >= 14.0
        )

        if should_split:
            text = " ".join(w["word"] for w in seg_words).strip()
            if text:
                segments.append(
                    {
                        "start": round(seg_words[0]["start"], 3),
                        "end": round(seg_words[-1]["end"], 3),
                        "text": text,
                        "speaker": speaker,
                        "language": language,
                        "fileName": file_name,
                        "trackIndex": track_index,
                    }
                )
            seg_words = [word]
        else:
            seg_words.append(word)

    if seg_words:
        text = " ".join(w["word"] for w in seg_words).strip()
        if text:
            segments.append(
                {
                    "start": round(seg_words[0]["start"], 3),
                    "end": round(seg_words[-1]["end"], 3),
                    "text": text,
                    "speaker": speaker,
                    "language": language,
                    "fileName": file_name,
                    "trackIndex": track_index,
                }
            )

    return segments


def format_timestamp_txt(seconds: float) -> str:
    ms_total = max(0, int(round(seconds * 1000)))
    hours, rem = divmod(ms_total, 3_600_000)
    minutes, rem = divmod(rem, 60_000)
    secs, ms = divmod(rem, 1000)
    return f"{hours:02d}:{minutes:02d}:{secs:02d}.{ms:03d}"


def format_timestamp_srt(seconds: float) -> str:
    ms_total = max(0, int(round(seconds * 1000)))
    hours, rem = divmod(ms_total, 3_600_000)
    minutes, rem = divmod(rem, 60_000)
    secs, ms = divmod(rem, 1000)
    return f"{hours:02d}:{minutes:02d}:{secs:02d},{ms:03d}"


def render_txt(segments: list[dict]) -> str:
    lines = [
        f"[{format_timestamp_txt(seg['start'])} - {format_timestamp_txt(seg['end'])}] {seg['speaker']}: {seg['text']}"
        for seg in segments
    ]
    return "\n".join(lines) + ("\n" if lines else "")


def render_srt(segments: list[dict]) -> str:
    blocks: list[str] = []
    for idx, seg in enumerate(segments, start=1):
        start_sec = max(0.0, seg["start"])
        end_sec = max(start_sec + 0.05, seg["end"])
        blocks.append(
            f"{idx}\n"
            f"{format_timestamp_srt(start_sec)} --> {format_timestamp_srt(end_sec)}\n"
            f"{seg['speaker']}: {seg['text']}"
        )
    return "\n\n".join(blocks) + ("\n" if blocks else "")


def render_output(
    segments: list[dict],
    fmt: str,
    model_id: str,
    extra_meta: dict | None = None,
) -> str:
    if fmt == "srt":
        return render_srt(segments)
    if fmt == "json":
        payload = {
            "modelId": model_id,
            "format": "json",
            **(extra_meta or {}),
            "segments": segments,
            "fullText": "\n".join(f"{seg['speaker']}: {seg['text']}" for seg in segments),
        }
        return json.dumps(payload, ensure_ascii=False, indent=2) + "\n"
    return render_txt(segments)


def run_job(job: dict) -> None:
    ensure_ml_imports()

    ffmpeg_path = job["ffmpegPath"]
    model_id = job.get("modelId", "ihnatov-yuri/parakeet-tdt-0.6b-v3-uk")
    default_language = job.get("defaultLanguage", "uk")
    fmt = job.get("format", "txt")
    merge_outputs = bool(job.get("mergeOutputs", False))
    inputs = job["inputs"]

    processor, model, device = load_model_and_processor(model_id)
    language_masks = build_language_masks(processor, device)
    active_mask: dict[str, Any] = {"ids": None}

    def apply_mask_hook(_module, _inputs, output):
        mask_idx = active_mask["ids"]
        if mask_idx is not None and mask_idx.numel() > 0:
            output = output.clone()
            output[..., mask_idx] = -1e9
        return output

    hook_handle = model.joint.head.register_forward_hook(apply_mask_hook)

    try:
        total_seconds = sum(max(0.0, float(item.get("duration") or 0.0)) for item in inputs)
        if total_seconds <= 0.0:
            total_seconds = float(len(inputs) or 1)

        emit_progress(0.0, total_seconds)

        all_segments: list[dict] = []
        cumulative_before = 0.0

        for pos, item in enumerate(inputs, start=1):
            track_idx = int(item.get("index", pos - 1))
            speaker = (item.get("speaker") or "").strip() or f"Мовець {track_idx + 1}"
            language = (item.get("language") or default_language or "uk").strip()
            mask_tensor = language_masks.get(language, language_masks.get("uk"))
            track_dur_hint = max(0.0, float(item.get("duration") or 0.0))

            emit_log(
                f"Доріжка {pos}/{len(inputs)}: «{item['name']}» (мовець: {speaker}, мова: {language})..."
            )

            tokens, actual_dur = transcribe_track_streaming(
                ffmpeg_path=ffmpeg_path,
                input_path=item["path"],
                speaker=speaker,
                track_dur_hint=track_dur_hint,
                processor=processor,
                model=model,
                device=device,
                active_mask_holder=active_mask,
                mask_tensor=mask_tensor,
                cumulative_before=cumulative_before,
                total_seconds=total_seconds,
            )
            cumulative_before += track_dur_hint if track_dur_hint > 0.0 else actual_dur

            track_segments = tokens_to_segments(
                tokens=tokens,
                speaker=speaker,
                language=language,
                file_name=item["name"],
                track_index=track_idx,
            )
            all_segments.extend(track_segments)

            individual_content = render_output(
                segments=track_segments,
                fmt=fmt,
                model_id=model_id,
                extra_meta={
                    "speaker": speaker,
                    "language": language,
                    "fileName": item["name"],
                    "sourcePath": item["path"],
                },
            )
            Path(item["individualPartialPath"]).write_text(
                individual_content, encoding="utf-8"
            )
            emit_log(
                f"Доріжку {pos}/{len(inputs)} ({speaker}) завершено. Збережено сегментів: {len(track_segments)}."
            )

        all_segments.sort(
            key=lambda seg: (round(seg["start"], 3), round(seg["end"], 3), seg["trackIndex"])
        )

        merged_partial = job.get("mergedPartialPath")
        if merge_outputs and merged_partial:
            emit_log(f"Об'єднання {len(all_segments)} сегментів у спільний файл...")
            merged_content = render_output(
                segments=all_segments,
                fmt=fmt,
                model_id=model_id,
                extra_meta={
                    "trackCount": len(inputs),
                    "tracks": [
                        {
                            "index": int(item.get("index", idx)),
                            "fileName": item["name"],
                            "speaker": (item.get("speaker") or "").strip()
                            or f"Мовець {int(item.get('index', idx)) + 1}",
                            "language": (item.get("language") or default_language or "uk").strip(),
                        }
                        for idx, item in enumerate(inputs)
                    ],
                },
            )
            Path(merged_partial).write_text(merged_content, encoding="utf-8")

        preview_path = job.get("previewPath")
        if preview_path:
            preview_lines = [
                f"[{format_timestamp_txt(seg['start'])} - {format_timestamp_txt(seg['end'])}] {seg['speaker']}: {seg['text']}"
                for seg in all_segments[:200]
            ]
            if len(all_segments) > 200:
                preview_lines.append(
                    f"... (+{len(all_segments) - 200} more segments in output file)"
                )
            Path(preview_path).write_text("\n".join(preview_lines), encoding="utf-8")

        emit_progress(total_seconds, total_seconds)
        emit_log("Транскрибцію успішно завершено.")
    finally:
        hook_handle.remove()


def main() -> int:
    configure_stdio()
    parser = argparse.ArgumentParser(description="Multi-track Parakeet TDT transcription worker")
    parser.add_argument("--job-file", required=True, help="Path to UTF-8 JSON job specification")
    args = parser.parse_args()

    job_path = Path(args.job_file)
    job = json.loads(job_path.read_text(encoding="utf-8"))
    run_job(job)
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
