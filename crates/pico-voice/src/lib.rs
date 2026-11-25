pub mod pipeline;
pub mod stt;
pub mod tts;
pub mod vad;

pub use pipeline::VoicePipeline;
pub use stt::{PlatformSpeechRecognizer, SpeechNormalizer, WindowsSpeechRecognizer};
pub use tts::{
    decode_wav_to_synthesized, encode_synthesized_to_wav, MockSpeechSynthesizer,
    NativeLinuxSynthesizer, NativeWindowsSynthesizer, PiperSynthesizer, PlatformSpeechSynthesizer,
    SpeechPreprocessor, WindowsSpeechSynthesizer,
};
pub use vad::{VadProcessor, VadState};

#[cfg(test)]
mod tests {
    use super::*;
    use pico_core::config::VoiceConfig;
    use tokio::sync::broadcast;

    #[test]
    fn test_voice_config_validation_rejects_invalid_engines() {
        let config = VoiceConfig::default();
        assert!(config.validate().is_ok());

        let bad_stt = VoiceConfig {
            stt_engine: "invalid_stt_engine_xyz".into(),
            ..VoiceConfig::default()
        };
        assert!(bad_stt.validate().is_err());

        let bad_tts = VoiceConfig {
            tts_engine: "invalid_tts_engine_xyz".into(),
            ..VoiceConfig::default()
        };
        assert!(bad_tts.validate().is_err());

        let winrt_cfg = VoiceConfig {
            tts_engine: "winrt".into(),
            ..VoiceConfig::default()
        };
        assert!(winrt_cfg.validate().is_ok());
    }

    #[test]
    fn test_stt_tts_wiring_from_config() {
        let (tx, _rx) = broadcast::channel(16);
        let config = VoiceConfig {
            stt_engine: "local".into(),
            tts_engine: "native_sapi".into(),
            voice_name: Some("TestVoice".into()),
            speech_rate: 1.25,
            ..VoiceConfig::default()
        };

        let pipeline = VoicePipeline::new_headless(config.clone(), tx).unwrap();
        assert_eq!(pipeline.stt_engine(), "local");
        let recognizer = PlatformSpeechRecognizer::from_config(&config);
        assert_eq!(recognizer.stt_engine(), "local");

        let synth = PlatformSpeechSynthesizer::from_config(&config);
        assert_eq!(synth.tts_engine(), "native_sapi");
    }
}
