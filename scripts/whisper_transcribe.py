import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))
from asr_service import load_wav_mono_16k  # noqa: E402


def transcribe(audio_path: str) -> str:
    samples, _sr = load_wav_mono_16k(audio_path)
    if len(samples) == 0:
        return ""
    try:
        from faster_whisper import WhisperModel
        model = WhisperModel("tiny.en", device="cpu", compute_type="int8")
        segments, _ = model.transcribe(samples, beam_size=1, language="en")
        return " ".join(s.text for s in segments).strip()
    except ImportError:
        import whisper
        model = whisper.load_model("tiny.en")
        result = model.transcribe(samples)
        return result.get("text", "").strip()


if __name__ == "__main__":
    if len(sys.argv) < 2:
        sys.stderr.write("Usage: whisper_transcribe.py <wav_path>\n")
        sys.exit(2)
    try:
        print(transcribe(sys.argv[1]), flush=True)
    except Exception as exc:
        print(f"ERROR:{exc}", flush=True)
        sys.exit(1)
