import argparse
import os
import sys
import wave
from pathlib import Path

import numpy as np

# Ensure scripts directory is importable regardless of cwd.
SCRIPT_DIR = Path(__file__).resolve().parent
MODELS_ROOT = SCRIPT_DIR.parent / "models"


def load_wav_mono_16k(wav_path: str) -> tuple[np.ndarray, int]:
    """Reads any standard uncompressed PCM WAV (8/16/24/32-bit, mono or multi-channel) as 16kHz float32."""
    p = Path(wav_path)
    if not p.is_file():
        raise FileNotFoundError(f"WAV file does not exist: {wav_path}")

    with wave.open(str(p), "rb") as wf:
        sr = wf.getframerate()
        n_channels = wf.getnchannels()
        sampwidth = wf.getsampwidth()
        n_frames = wf.getnframes()
        raw = wf.readframes(n_frames)

    if not raw or n_frames == 0:
        return np.zeros(0, dtype=np.float32), 16000

    if sampwidth == 1:
        samples = (np.frombuffer(raw, dtype=np.uint8).astype(np.float32) - 128.0) / 128.0
    elif sampwidth == 2:
        samples = np.frombuffer(raw, dtype=np.int16).astype(np.float32) / 32768.0
    elif sampwidth == 3:
        b = np.frombuffer(raw, dtype=np.uint8).reshape(-1, 3)
        ints = (
            b[:, 0].astype(np.int32)
            | (b[:, 1].astype(np.int32) << 8)
            | (b[:, 2].astype(np.int32) << 16)
        )
        ints = np.where(ints & 0x800000, ints - 0x1000000, ints)
        samples = ints.astype(np.float32) / 8388608.0
    elif sampwidth == 4:
        samples = np.frombuffer(raw, dtype=np.int32).astype(np.float32) / 2147483648.0
    else:
        raise ValueError(f"Unsupported WAV sample width: {sampwidth} bytes")

    if n_channels > 1:
        usable = (len(samples) // n_channels) * n_channels
        samples = samples[:usable].reshape(-1, n_channels).mean(axis=1)

    if sr != 16000 and len(samples) > 1:
        duration = len(samples) / float(sr)
        target_len = max(1, int(round(duration * 16000.0)))
        old_idx = np.linspace(0.0, len(samples) - 1, num=len(samples))
        new_idx = np.linspace(0.0, len(samples) - 1, num=target_len)
        samples = np.interp(new_idx, old_idx, samples).astype(np.float32)
        sr = 16000

    return np.ascontiguousarray(samples, dtype=np.float32), sr


def get_system_ram_gb() -> float:
    try:
        import psutil
        return psutil.virtual_memory().total / (1024**3)
    except Exception:
        return 8.0


def find_model_dir(prefixes: list[str]) -> Path | None:
    search_roots = [Path("models"), MODELS_ROOT]
    for root in search_roots:
        if not root.is_dir():
            continue
        for prefix in prefixes:
            for item in sorted(root.iterdir()):
                if item.is_dir() and prefix in item.name.lower():
                    return item
    return None


def resolve_engine(engine_arg: str) -> str:
    req = engine_arg.lower()
    if req in ("parakeet", "tdt"):
        return "parakeet"
    if req in ("moonshine", "whisper", "local"):
        return "moonshine" if find_model_dir(["moonshine"]) else req

    ram = get_system_ram_gb()
    has_parakeet = find_model_dir(["parakeet", "nemo"]) is not None
    has_moonshine = find_model_dir(["moonshine"]) is not None

    if has_parakeet and (ram >= 10.0 or not has_moonshine):
        return "parakeet"
    if has_moonshine:
        return "moonshine"
    return "whisper"


def create_recognizer(engine: str):
    import sherpa_onnx

    threads = max(1, min(4, (os.cpu_count() or 2) // 2))

    if engine == "parakeet":
        m_dir = find_model_dir(["parakeet", "nemo"])
        if m_dir is None:
            raise FileNotFoundError("Parakeet model directory not found under models/")
        enc = list(m_dir.glob("*encoder*.onnx"))
        dec = list(m_dir.glob("*decoder*.onnx"))
        join = list(m_dir.glob("*joiner*.onnx"))
        tok = m_dir / "tokens.txt"
        if not (enc and dec and join and tok.exists()):
            raise FileNotFoundError(f"Incomplete Parakeet files in {m_dir}")
        rec = sherpa_onnx.OfflineRecognizer.from_transducer(
            encoder=str(enc[0]),
            decoder=str(dec[0]),
            joiner=str(join[0]),
            tokens=str(tok),
            num_threads=threads,
            sample_rate=16000,
            feature_dim=80,
            decoding_method="greedy_search",
            model_type="nemo_transducer",
        )
        return "sherpa_offline", rec, "Parakeet-TDT-0.6B"

    if engine == "moonshine":
        m_dir = find_model_dir(["moonshine"])
        if m_dir is None:
            raise FileNotFoundError("Moonshine model directory not found under models/")
        prep = list(m_dir.glob("*preprocess*.onnx"))
        enc = list(m_dir.glob("*encode*.onnx"))
        unc_dec = list(m_dir.glob("*uncached_decode*.onnx"))
        cac_dec = list(m_dir.glob("*cached_decode*.onnx"))
        tok = m_dir / "tokens.txt"
        if not (prep and enc and unc_dec and cac_dec and tok.exists()):
            raise FileNotFoundError(f"Incomplete Moonshine files in {m_dir}")
        rec = sherpa_onnx.OfflineRecognizer.from_moonshine(
            preprocessor=str(prep[0]),
            encoder=str(enc[0]),
            uncached_decoder=str(unc_dec[0]),
            cached_decoder=str(cac_dec[0]),
            tokens=str(tok),
            num_threads=threads,
        )
        return "sherpa_offline", rec, "Moonshine-Tiny-ONNX"

    raise ValueError(f"Unsupported sherpa ASR engine: {engine}")


def create_whisper_fallback():
    from faster_whisper import WhisperModel
    model = WhisperModel("tiny.en", device="cpu", compute_type="int8")
    return "faster_whisper", model, "Faster-Whisper-Tiny"


def transcribe_once(backend, samples: np.ndarray, sample_rate: int) -> str:
    if len(samples) == 0:
        return ""
    kind, rec, _ = backend
    if kind == "sherpa_offline":
        stream = rec.create_stream()
        stream.accept_waveform(sample_rate, samples)
        rec.decode_stream(stream)
        return stream.result.text.strip()
    if kind == "faster_whisper":
        segments, _ = rec.transcribe(samples, beam_size=1, language="en")
        return " ".join(seg.text for seg in segments).strip()
    return ""


def init_backend(engine_arg: str):
    target = resolve_engine(engine_arg)
    if target in ("parakeet", "moonshine"):
        try:
            return create_recognizer(target)
        except Exception as primary_err:
            sys.stderr.write(f"[asr_service] {target} init failed ({primary_err}); trying whisper fallback\n")
            sys.stderr.flush()
    return create_whisper_fallback()


def main():
    parser = argparse.ArgumentParser(description="Pico Local Offline Multi-Tier ASR Service")
    parser.add_argument("audio_path", nargs="?", help="WAV file path for single-file transcription")
    parser.add_argument("--daemon", action="store_true", help="Run as long-lived stdin/stdout ASR daemon")
    parser.add_argument("--engine", default="auto", help="auto | parakeet | moonshine | whisper")
    args = parser.parse_args()

    try:
        backend = init_backend(args.engine)
    except Exception as exc:
        print(f"INIT_ERROR: {exc}", flush=True)
        sys.exit(1)

    if args.daemon:
        print(f"READY:{backend[2]}", flush=True)
        for line in sys.stdin:
            wav_path = line.strip()
            if wav_path == "QUIT":
                break
            if not wav_path:
                continue
            try:
                samples, sr = load_wav_mono_16k(wav_path)
                print(transcribe_once(backend, samples, sr), flush=True)
            except Exception as exc:
                print(f"ERROR:{exc}", flush=True)
    elif args.audio_path:
        try:
            samples, sr = load_wav_mono_16k(args.audio_path)
            print(transcribe_once(backend, samples, sr), flush=True)
        except Exception as exc:
            print(f"ERROR:{exc}", flush=True)
            sys.exit(1)
    else:
        parser.print_help()
        sys.exit(2)


if __name__ == "__main__":
    main()
