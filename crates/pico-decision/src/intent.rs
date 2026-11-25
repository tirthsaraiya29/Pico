//! Non-generative intent classification and capability affinity mapping.

use serde::{Deserialize, Serialize};

/// High-level user objective intent categories.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum UserIntent {
    SystemDiagnostic,
    ProcessManagement,
    FilesystemOperation,
    MediaAudioControl,
    DisplayControl,
    PowerManagement,
    NetworkUtility,
    ArithmeticComputation,
    ApplicationLaunch,
    ApplicationClose,
    TimerReminder,
    ClipboardOperation,
    ServiceManagement,
    PackageManagement,
    DirectConversation,
    Unknown,
}

impl UserIntent {
    pub fn as_str(&self) -> &'static str {
        match self {
            UserIntent::SystemDiagnostic => "system_diagnostic",
            UserIntent::ProcessManagement => "process_management",
            UserIntent::FilesystemOperation => "filesystem_operation",
            UserIntent::MediaAudioControl => "media_audio_control",
            UserIntent::DisplayControl => "display_control",
            UserIntent::PowerManagement => "power_management",
            UserIntent::NetworkUtility => "network_utility",
            UserIntent::ArithmeticComputation => "arithmetic_computation",
            UserIntent::ApplicationLaunch => "application_launch",
            UserIntent::ApplicationClose => "application_close",
            UserIntent::TimerReminder => "timer_reminder",
            UserIntent::ClipboardOperation => "clipboard_operation",
            UserIntent::ServiceManagement => "service_management",
            UserIntent::PackageManagement => "package_management",
            UserIntent::DirectConversation => "direct_conversation",
            UserIntent::Unknown => "unknown",
        }
    }

    /// Evaluates category affinity between this detected intent and a tool's registered category.
    pub fn affinity_for_category(&self, tool_category: &str) -> f32 {
        let cat = tool_category.to_lowercase();
        match self {
            UserIntent::SystemDiagnostic => {
                if cat.contains("system")
                    || cat.contains("sys")
                    || cat.contains("hardware")
                    || cat.contains("diagnostic")
                {
                    1.0
                } else if cat.contains("process") {
                    0.5
                } else {
                    0.1
                }
            }
            UserIntent::ProcessManagement => {
                if cat.contains("process") || cat.contains("task") {
                    1.0
                } else if cat.contains("application") || cat.contains("system") {
                    0.4
                } else {
                    0.1
                }
            }
            UserIntent::FilesystemOperation => {
                if cat.contains("file")
                    || cat.contains("fs")
                    || cat.contains("disk")
                    || cat.contains("storage")
                {
                    1.0
                } else {
                    0.1
                }
            }
            UserIntent::MediaAudioControl => {
                if cat.contains("audio")
                    || cat.contains("sound")
                    || cat.contains("media")
                    || cat.contains("volume")
                {
                    1.0
                } else {
                    0.05
                }
            }
            UserIntent::DisplayControl => {
                if cat.contains("display") || cat.contains("screen") || cat.contains("monitor") {
                    1.0
                } else {
                    0.05
                }
            }
            UserIntent::PowerManagement => {
                if cat.contains("power") || cat.contains("battery") {
                    1.0
                } else if cat.contains("system") {
                    0.4
                } else {
                    0.05
                }
            }
            UserIntent::NetworkUtility => {
                if cat.contains("net")
                    || cat.contains("web")
                    || cat.contains("dns")
                    || cat.contains("http")
                {
                    1.0
                } else {
                    0.1
                }
            }
            UserIntent::ArithmeticComputation => {
                if cat.contains("calc") || cat.contains("math") {
                    1.0
                } else {
                    0.05
                }
            }
            UserIntent::ApplicationLaunch => {
                if cat.contains("app") || cat.contains("launch") {
                    1.0
                } else if cat.contains("system") {
                    0.3
                } else {
                    0.05
                }
            }
            UserIntent::ApplicationClose => {
                if cat.contains("app") || cat.contains("process") {
                    1.0
                } else {
                    0.05
                }
            }
            UserIntent::TimerReminder => {
                if cat.contains("timer") || cat.contains("time") || cat.contains("scheduler") {
                    1.0
                } else {
                    0.05
                }
            }
            UserIntent::ClipboardOperation => {
                if cat.contains("clip") || cat.contains("clipboard") {
                    1.0
                } else {
                    0.05
                }
            }
            UserIntent::ServiceManagement => {
                if cat.contains("service") || cat.contains("system") {
                    1.0
                } else {
                    0.1
                }
            }
            UserIntent::PackageManagement => {
                if cat.contains("package") || cat.contains("winget") || cat.contains("apt") {
                    1.0
                } else {
                    0.1
                }
            }
            UserIntent::DirectConversation => 0.0,
            UserIntent::Unknown => 0.25,
        }
    }
}

/// Outcome of classifying user input intent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IntentClassification {
    pub primary_intent: UserIntent,
    pub confidence: f32,
    pub requires_tool: bool,
    pub secondary_intent: Option<UserIntent>,
}

/// High-speed non-generative intent classifier.
#[derive(Debug, Clone, Default)]
pub struct IntentClassifier;

impl IntentClassifier {
    pub fn new() -> Self {
        Self
    }

    /// Classifies user text into intent category and determines whether a tool execution is required.
    pub fn classify(&self, text: &str) -> IntentClassification {
        let trimmed = text.trim();
        let lower = trimmed.to_lowercase();

        // 1. Conversational greetings / identity / explicit non-task statements
        if is_direct_conversation(&lower) {
            return IntentClassification {
                primary_intent: UserIntent::DirectConversation,
                confidence: 0.95,
                requires_tool: false,
                secondary_intent: None,
            };
        }

        // 2. Display / Brightness Control
        if is_display_intent(&lower) {
            return IntentClassification {
                primary_intent: UserIntent::DisplayControl,
                confidence: 0.93,
                requires_tool: true,
                secondary_intent: None,
            };
        }

        // 3. Timers & Alarms (check before arithmetic/audio so "30 seconds" stays timer)
        if is_timer_intent(&lower) {
            return IntentClassification {
                primary_intent: UserIntent::TimerReminder,
                confidence: 0.94,
                requires_tool: true,
                secondary_intent: None,
            };
        }

        // 4. Math & Arithmetic
        if is_arithmetic(&lower) {
            return IntentClassification {
                primary_intent: UserIntent::ArithmeticComputation,
                confidence: 0.92,
                requires_tool: true,
                secondary_intent: None,
            };
        }

        // 5. Application Close / Quit / Exit (must precede launch & generic process checks)
        if is_app_close_intent(&lower) {
            return IntentClassification {
                primary_intent: UserIntent::ApplicationClose,
                confidence: 0.93,
                requires_tool: true,
                secondary_intent: Some(UserIntent::ProcessManagement),
            };
        }

        // 6. Process Management
        if is_process_intent(&lower) {
            return IntentClassification {
                primary_intent: UserIntent::ProcessManagement,
                confidence: 0.90,
                requires_tool: true,
                secondary_intent: Some(UserIntent::SystemDiagnostic),
            };
        }

        // 7. System Diagnostics & Hardware
        if is_system_intent(&lower) {
            return IntentClassification {
                primary_intent: UserIntent::SystemDiagnostic,
                confidence: 0.88,
                requires_tool: true,
                secondary_intent: None,
            };
        }

        // 8. Application Launch (Prioritize over filesystem for "open <app>" commands)
        if is_app_launch_intent(&lower) {
            return IntentClassification {
                primary_intent: UserIntent::ApplicationLaunch,
                confidence: 0.94,
                requires_tool: true,
                secondary_intent: None,
            };
        }

        // 9. Filesystem Operations
        if is_filesystem_intent(&lower) {
            return IntentClassification {
                primary_intent: UserIntent::FilesystemOperation,
                confidence: 0.89,
                requires_tool: true,
                secondary_intent: None,
            };
        }

        // 10. Media & Audio
        if is_audio_intent(&lower) {
            return IntentClassification {
                primary_intent: UserIntent::MediaAudioControl,
                confidence: 0.92,
                requires_tool: true,
                secondary_intent: None,
            };
        }

        // 11. Power Management
        if is_power_intent(&lower) {
            return IntentClassification {
                primary_intent: UserIntent::PowerManagement,
                confidence: 0.93,
                requires_tool: true,
                secondary_intent: Some(UserIntent::SystemDiagnostic),
            };
        }

        // 12. Clipboard
        if is_clipboard_intent(&lower) {
            return IntentClassification {
                primary_intent: UserIntent::ClipboardOperation,
                confidence: 0.91,
                requires_tool: true,
                secondary_intent: None,
            };
        }

        // 13. Network & Web Fetch
        if is_network_intent(&lower) {
            return IntentClassification {
                primary_intent: UserIntent::NetworkUtility,
                confidence: 0.87,
                requires_tool: true,
                secondary_intent: None,
            };
        }

        // Default / Unknown
        IntentClassification {
            primary_intent: UserIntent::Unknown,
            confidence: 0.40,
            requires_tool: true,
            secondary_intent: None,
        }
    }
}

fn is_direct_conversation(t: &str) -> bool {
    let lower = t.to_lowercase();
    let trimmed = lower.trim_matches(|c: char| c.is_ascii_punctuation() || c.is_whitespace());
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();

    if words.is_empty() {
        return true;
    }

    let greetings = [
        "hello",
        "hi",
        "hey",
        "greetings",
        "bye",
        "goodbye",
        "thanks",
    ];
    if words.iter().any(|w| greetings.contains(w)) && words.len() <= 6 {
        let task_keywords = [
            "process",
            "calc",
            "calculate",
            "file",
            "volume",
            "mute",
            "kill",
            "system",
            "ram",
            "timer",
            "brightness",
            "open",
            "close",
        ];
        if !words.iter().any(|w| task_keywords.contains(w)) {
            return true;
        }
    }

    let conversational_phrases = [
        "who are you",
        "what are you",
        "what can you do",
        "who created you",
        "how are you",
        "thank you",
        "good morning",
        "good afternoon",
        "good evening",
        "tell me a joke",
        "tell a joke",
        "i'm not sure",
        "im not sure",
        "not sure",
        "i don't know",
    ];
    if conversational_phrases.iter().any(|p| trimmed.contains(p)) {
        return true;
    }

    matches!(trimmed, "joke" | "cancel" | "nevermind" | "never mind")
}

fn is_display_intent(t: &str) -> bool {
    t.contains("brightness")
        || t.contains("backlight")
        || t.contains("dim screen")
        || t.contains("dim the screen")
        || t.contains("brighter")
        || t.contains("dimmer")
}

fn is_app_close_intent(t: &str) -> bool {
    let words: Vec<&str> = t
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|w| !w.is_empty())
        .collect();
    if words.len() >= 2 {
        let first = words[0];
        if matches!(first, "close" | "quit" | "exit") {
            let file_terms = ["file", "files", "folder", "directory"];
            if !words.iter().skip(1).any(|w| file_terms.contains(w)) {
                return true;
            }
        }
    }
    false
}

fn is_arithmetic(t: &str) -> bool {
    if t.contains("calculate")
        || t.contains("compute")
        || t.contains("solve")
        || t.contains("sqrt")
        || t.contains("math")
        || t.contains("eval")
    {
        return true;
    }
    // Check for math expressions like "25 * 4", "100 / 2", "12 + 45"
    let math_chars = ['+', '*', '/', '^', '%'];
    let has_math_op = math_chars.iter().any(|&c| t.contains(c));
    let has_digit = t.chars().any(|c| c.is_ascii_digit());
    has_math_op && has_digit
}

fn is_process_intent(t: &str) -> bool {
    t.contains("process")
        || t.contains("running app")
        || t.contains("kill")
        || t.contains("terminate")
        || t.contains("task manager")
        || t.contains("consuming memory")
        || t.contains("cpu usage")
        || t.contains("end task")
}

fn is_system_intent(t: &str) -> bool {
    t.contains("system info")
        || t.contains("system information")
        || t.contains("hardware")
        || t.contains("ram")
        || t.contains("specs")
        || t.contains("computer specs")
        || t.contains("memory available")
        || t.contains("os version")
        || t.contains("cpu stats")
        || t.contains("uptime")
        || t.contains("battery")
}

fn is_filesystem_intent(t: &str) -> bool {
    let lower = t.to_lowercase();
    if is_app_launch_intent(&lower) {
        return false;
    }
    lower.contains("file")
        || lower.contains("directory")
        || lower.contains("folder")
        || lower.contains("read file")
        || lower.contains("write to")
        || lower.contains("search for")
        || lower.contains("disk space")
        || lower.contains(".txt")
        || lower.contains(".json")
        || lower.contains(".rs")
        || lower.contains(".toml")
}

fn is_audio_intent(t: &str) -> bool {
    let words: Vec<&str> = t
        .split(|c: char| !c.is_alphanumeric())
        .filter(|w| !w.is_empty())
        .collect();
    words.iter().any(|w| {
        matches!(
            *w,
            "volume" | "mute" | "unmute" | "audio" | "sound" | "sounds" | "speaker" | "speakers"
        )
    })
}

fn is_power_intent(t: &str) -> bool {
    t.contains("shutdown")
        || t.contains("restart")
        || t.contains("reboot")
        || t.contains("sleep")
        || t.contains("lock screen")
        || t.contains("lock computer")
        || t.contains("power plan")
}

fn is_timer_intent(t: &str) -> bool {
    t.contains("timer")
        || t.contains("alarm")
        || t.contains("remind me in")
        || t.contains("countdown")
}

fn is_clipboard_intent(t: &str) -> bool {
    t.contains("clipboard") || t.contains("paste") || t.contains("copy to clipboard")
}

fn is_app_launch_intent(t: &str) -> bool {
    let lower = t.to_lowercase();
    let words: Vec<&str> = lower
        .split_whitespace()
        .map(|w| w.trim_matches(|c: char| !c.is_alphanumeric()))
        .filter(|w| !w.is_empty())
        .collect();

    if words.is_empty() {
        return false;
    }

    // Direct app launching commands: "open chrome", "launch brave", "start notepad"
    let launch_verbs = ["open", "launch", "start", "run"];
    if let Some(&first) = words.first() {
        if launch_verbs.contains(&first) {
            // Check if explicitly asking for file/folder/directory operations
            let is_file_term = words.iter().skip(1).any(|&w| {
                w == "file"
                    || w == "files"
                    || w == "folder"
                    || w == "directory"
                    || w.ends_with(".txt")
                    || w.ends_with(".rs")
                    || w.ends_with(".json")
                    || w.ends_with(".log")
            });
            if !is_file_term {
                return true;
            }
        }
    }

    lower.contains("open app")
        || lower.contains("open browser")
        || lower.contains("open program")
        || lower.contains("launch app")
        || lower.contains("open chrome")
        || lower.contains("open brave")
        || lower.contains("open notepad")
        || lower.contains("open calc")
}

fn is_network_intent(t: &str) -> bool {
    t.contains("ping")
        || t.contains("ip address")
        || t.contains("network")
        || t.contains("dns")
        || t.contains("fetch")
        || t.contains("curl")
        || t.contains("website")
        || t.contains("http://")
        || t.contains("https://")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_intent_classification() {
        let classifier = IntentClassifier::new();

        assert_eq!(
            classifier
                .classify("Find which processes are using the most RAM")
                .primary_intent,
            UserIntent::ProcessManagement
        );

        assert_eq!(
            classifier
                .classify("What is 15 * 8 + sqrt(144)?")
                .primary_intent,
            UserIntent::ArithmeticComputation
        );

        assert_eq!(
            classifier
                .classify("Show my system hardware information")
                .primary_intent,
            UserIntent::SystemDiagnostic
        );

        assert_eq!(
            classifier
                .classify("Hello Pico, who are you?")
                .primary_intent,
            UserIntent::DirectConversation
        );

        assert_eq!(
            classifier
                .classify("Set volume to 50 percent")
                .primary_intent,
            UserIntent::MediaAudioControl
        );

        assert_eq!(
            classifier
                .classify("Set a timer for 30 seconds")
                .primary_intent,
            UserIntent::TimerReminder
        );

        assert_eq!(
            classifier.classify("Close brave").primary_intent,
            UserIntent::ApplicationClose
        );

        assert_eq!(
            classifier.classify("increase brightness").primary_intent,
            UserIntent::DisplayControl
        );

        assert_eq!(
            classifier.classify("Mute sounds").primary_intent,
            UserIntent::MediaAudioControl
        );

        assert_eq!(
            classifier.classify("first").primary_intent,
            UserIntent::Unknown
        );
    }
}
