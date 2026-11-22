pub mod pipeline;
pub mod stt;
pub mod tts;
pub mod vad;

pub use pipeline::VoicePipeline;
pub use stt::{PlatformSpeechRecognizer, WindowsSpeechRecognizer};
pub use tts::{NativeLinuxSynthesizer, NativeWindowsSynthesizer, PlatformSpeechSynthesizer};
pub use vad::{VadProcessor, VadState};

#[cfg(test)]
mod tests {
    use super::*;
    use pico_core::config::VoiceConfig;

    #[test]
    fn test_voice_config_validation_valid() {
        let config = VoiceConfig::default();
        assert!(config.validate().is_ok());
    }

    #[test]
    fn test_voice_config_validation_invalid_engines() {
        let config = VoiceConfig {
            stt_engine: "invalid_stt_engine_xyz".into(),
            ..Default::default()
        };
        assert!(config.validate().is_err());

        let config = VoiceConfig {
            tts_engine: "invalid_tts_engine_xyz".into(),
            ..Default::default()
        };
        assert!(config.validate().is_err());

        let config = VoiceConfig {
            speech_rate: 10.0,
            ..Default::default()
        };
        assert!(config.validate().is_err());
    }

    #[test]
    fn test_stt_tts_wiring_from_config() {
        let config = VoiceConfig {
            stt_engine: "local".into(),
            tts_engine: "native_sapi".into(),
            speech_rate: 1.25,
            ..Default::default()
        };

        let recognizer = PlatformSpeechRecognizer::from_config(&config);
        assert_eq!(recognizer.stt_engine(), "local");

        let synth = PlatformSpeechSynthesizer::from_config(&config);
        assert_eq!(synth.tts_engine(), "native_sapi");
    }
}
