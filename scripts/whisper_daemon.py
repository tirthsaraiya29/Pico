import sys
from pathlib import Path

# Delegate cleanly to the hardened multi-format asr_service loader & transcriber.
sys.path.insert(0, str(Path(__file__).resolve().parent))
from asr_service import load_wav_mono_16k  # noqa: E402


def load_whisper():
    from faster_whisper import WhisperModel
    return WhisperModel("tiny.en", device="cpu", compute_type="int8")


def main():
    try:
        model = load_whisper()
        print("READY", flush=True)
    except Exception as exc:
        print(f"INIT_ERROR:{exc}", flush=True)
        sys.exit(1)

    for line in sys.stdin:
        wav_path = line.strip()
        if wav_path == "QUIT":
            break
        if not wav_path:
            continue
        try:
            samples, _sr = load_wav_mono_16k(wav_path)
            if len(samples) == 0:
                print("", flush=True)
                continue
            segments, _ = model.transcribe(samples, beam_size=1, language="en")
            print(" ".join(seg.text for seg in segments).strip(), flush=True)
        except Exception as exc:
            print(f"ERROR:{exc}", flush=True)


if __name__ == "__main__":
    main()
