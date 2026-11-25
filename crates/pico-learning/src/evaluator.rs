//! Held-out benchmark evaluation suite and model promotion gate.
//! Evaluates candidate ranking models against 50+ diverse test cases across all 22 operational domains.

use pico_core::error::Result;
use pico_core::traits::{CandidateAction, PermissionTier};
use pico_inference::ranking::{MultiFeatureRanker, RankingWeights};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone)]
pub struct BenchmarkItem {
    pub query: &'static str,
    pub expected_action: &'static str,
    pub candidate_actions: Vec<CandidateAction>,
    pub intent_affinities: Vec<f32>,
    pub historical_scores: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EvaluationMetrics {
    pub top1_accuracy: f32,
    pub top3_recall: f32,
    pub total_samples: usize,
    pub passed_promotion: bool,
}

pub struct ModelEvaluator {
    benchmark_dataset: Vec<BenchmarkItem>,
}

impl Default for ModelEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

impl ModelEvaluator {
    pub fn new() -> Self {
        Self {
            benchmark_dataset: build_standard_benchmark(),
        }
    }

    /// Number of held-out benchmark items.
    pub fn sample_count(&self) -> usize {
        self.benchmark_dataset.len()
    }

    /// Evaluates candidate weights against the held-out benchmark suite without oracle label leakage.
    pub fn evaluate(
        &self,
        weights: &RankingWeights,
        baseline_top1: f32,
    ) -> Result<EvaluationMetrics> {
        let ranker = MultiFeatureRanker::new(weights.clone());
        let mut top1_correct = 0usize;
        let mut top3_correct = 0usize;
        let total = self.benchmark_dataset.len();

        for item in &self.benchmark_dataset {
            let inferred_affinities: Vec<f32> = item
                .candidate_actions
                .iter()
                .map(|c| infer_query_category_affinity(item.query, &c.category))
                .collect();
            let neutral_history = vec![0.5f32; item.candidate_actions.len()];

            let ranked = ranker.rank_candidates(
                item.query,
                &item.candidate_actions,
                &inferred_affinities,
                &neutral_history,
                1.0,
            );

            if let Some(first) = ranked.first() {
                if first.action_name == item.expected_action {
                    top1_correct += 1;
                }
            }

            let top3_names: Vec<&str> = ranked
                .iter()
                .take(3)
                .map(|c| c.action_name.as_str())
                .collect();
            if top3_names.contains(&item.expected_action) {
                top3_correct += 1;
            }
        }

        let top1_accuracy = if total > 0 {
            top1_correct as f32 / total as f32
        } else {
            0.0
        };

        let top3_recall = if total > 0 {
            top3_correct as f32 / total as f32
        } else {
            0.0
        };

        let passed_promotion =
            top1_accuracy >= (baseline_top1 - 0.02) && top3_recall >= 0.80 && top1_accuracy >= 0.70;

        Ok(EvaluationMetrics {
            top1_accuracy,
            top3_recall,
            total_samples: total,
            passed_promotion,
        })
    }
}

/// Computes position-invariant domain affinity directly from query tokens and candidate category.
fn infer_query_category_affinity(query: &str, category: &str) -> f32 {
    let q = query.to_lowercase();
    let cat = category.to_lowercase();
    let matches_category = match cat.as_str() {
        "filesystem" | "storage" | "backup" => {
            q.contains("file")
                || q.contains("disk")
                || q.contains("directory")
                || q.contains("folder")
                || q.contains("write")
                || q.contains("read")
                || q.contains("backup")
                || q.contains("snapshot")
                || q.contains("storage")
        }
        "process" => {
            q.contains("process")
                || q.contains("kill")
                || q.contains("cpu")
                || q.contains("ram")
                || q.contains("task")
        }
        "system" => {
            q.contains("system")
                || q.contains("os ")
                || q.contains("hardware")
                || q.contains("uptime")
                || q.contains("battery")
                || q.contains("specs")
        }
        "audio" => {
            q.contains("volume")
                || q.contains("mute")
                || q.contains("sound")
                || q.contains("audio")
                || q.contains("speaker")
        }
        "power" => {
            q.contains("sleep")
                || q.contains("shutdown")
                || q.contains("restart")
                || q.contains("lock")
                || q.contains("power")
                || q.contains("hibernat")
        }
        "network" => {
            q.contains("ping")
                || q.contains("network")
                || q.contains("dns")
                || q.contains("wifi")
                || q.contains("port")
                || q.contains("http")
                || q.contains("ip")
        }
        "calculator" => {
            q.contains("calculate") || q.contains("sqrt") || q.contains('*') || q.contains('+')
        }
        "timer" => q.contains("timer") || q.contains("alarm") || q.contains("remind"),
        "clipboard" => q.contains("clipboard") || q.contains("copy") || q.contains("paste"),
        "display" => {
            q.contains("brightness")
                || q.contains("screen")
                || q.contains("monitor")
                || q.contains("display")
        }
        "services" => q.contains("service") || q.contains("daemon"),
        "packages" => q.contains("package") || q.contains("install") || q.contains("upgrade"),
        "security" => {
            q.contains("firewall")
                || q.contains("defender")
                || q.contains("security")
                || q.contains("scan")
        }
        "environment" => {
            q.contains("env") || q.contains("environment") || q.contains("path variable")
        }
        "logs" => q.contains("log") || q.contains("journal") || q.contains("event"),
        "device" | "usb" => {
            q.contains("usb")
                || q.contains("device")
                || q.contains("driver")
                || q.contains("printer")
        }
        "app" | "application" => {
            q.contains("open ") || q.contains("launch ") || q.contains("close ")
        }
        "window_mgmt" => q.contains("window") || q.contains("focus") || q.contains("minimize"),
        "bluetooth" => q.contains("bluetooth") || q.contains("paired") || q.contains("headphones"),
        "notifications" => q.contains("notification") || q.contains("toast"),
        _ => false,
    };
    if matches_category {
        0.95
    } else {
        0.15
    }
}

fn build_standard_benchmark() -> Vec<BenchmarkItem> {
    let make_action = |name: &'static str, desc: &'static str, cat: &'static str| CandidateAction {
        name: name.to_string(),
        description: desc.to_string(),
        category: cat.to_string(),
        permission_tier: PermissionTier::ReadOnly,
        parameters_schema: serde_json::json!({}),
    };

    let make_item = |q: &'static str,
                     exp: &'static str,
                     target: CandidateAction,
                     dist1: CandidateAction,
                     dist2: CandidateAction| BenchmarkItem {
        query: q,
        expected_action: exp,
        // Put target in the middle so positional bias cannot inflate top-1 accuracy
        candidate_actions: vec![dist1, target, dist2],
        intent_affinities: vec![],
        historical_scores: vec![],
    };

    vec![
        // 1. Filesystem
        make_item(
            "Read contents of notes.txt",
            "fs.read_file",
            make_action(
                "fs.read_file",
                "Reads file contents from disk within workspace",
                "filesystem",
            ),
            make_action("media.volume", "Sets master audio volume", "audio"),
            make_action(
                "calculator",
                "Evaluates arithmetic math expressions",
                "calculator",
            ),
        ),
        make_item(
            "Write hello world into output.log",
            "fs.write_file",
            make_action(
                "fs.write_file",
                "Writes or appends content to a file",
                "filesystem",
            ),
            make_action("process.kill", "Terminates a running process", "process"),
            make_action("sys.info", "Displays system hardware specs", "system"),
        ),
        make_item(
            "List all files and subdirectories here",
            "fs.list_directory",
            make_action(
                "fs.list_directory",
                "Lists files and folders with sizes",
                "filesystem",
            ),
            make_action(
                "clipboard.get",
                "Retrieves text from clipboard",
                "clipboard",
            ),
            make_action("network.ping", "Sends ICMP echo request to host", "network"),
        ),
        make_item(
            "Delete temporary cache file temp.dat",
            "fs.delete_file",
            make_action(
                "fs.delete_file",
                "Removes a file from the filesystem",
                "filesystem",
            ),
            make_action("power.sleep", "Suspends operating system to sleep", "power"),
            make_action("timer.set", "Creates a countdown reminder timer", "timer"),
        ),
        // 2. Process
        make_item(
            "Kill unresponsive process notepad.exe",
            "process.kill",
            make_action(
                "process.kill",
                "Terminates a process by name or PID",
                "process",
            ),
            make_action("fs.read_file", "Reads text from disk file", "filesystem"),
            make_action("calculator", "Evaluates math arithmetic", "calculator"),
        ),
        make_item(
            "List all active running processes",
            "process.list",
            make_action(
                "process.list",
                "Enumerates active processes and memory usage",
                "process",
            ),
            make_action(
                "display.brightness",
                "Adjusts screen display brightness",
                "display",
            ),
            make_action("media.mute", "Mutes system audio speaker output", "audio"),
        ),
        make_item(
            "Find which process consumes the most CPU",
            "process.top_cpu",
            make_action(
                "process.top_cpu",
                "Lists processes sorted by CPU load",
                "process",
            ),
            make_action(
                "clipboard.set",
                "Copies text to system clipboard",
                "clipboard",
            ),
            make_action("timer.set", "Sets a countdown timer", "timer"),
        ),
        // 3. System
        make_item(
            "How much RAM memory is currently used",
            "sys.diagnostic",
            make_action(
                "sys.diagnostic",
                "Inspects CPU load and RAM memory utilization",
                "system",
            ),
            make_action("fs.write_file", "Writes text to file", "filesystem"),
            make_action("media.volume", "Adjusts audio volume", "audio"),
        ),
        make_item(
            "What is the system uptime",
            "sys.uptime",
            make_action(
                "sys.uptime",
                "Reports how long the system has been running",
                "system",
            ),
            make_action("power.restart", "Restarts the computer", "power"),
            make_action(
                "calculator",
                "Computes arithmetic math formulas",
                "calculator",
            ),
        ),
        make_item(
            "Show operating system version and host info",
            "sys.info",
            make_action(
                "sys.info",
                "Displays OS kernel version and machine architecture",
                "system",
            ),
            make_action("process.kill", "Stops running task process", "process"),
            make_action("network.ping", "Tests network reachability", "network"),
        ),
        // 4. Calculator
        make_item(
            "What is 15 * 12",
            "calculator",
            make_action(
                "calculator",
                "Evaluates mathematical arithmetic expressions",
                "calculator",
            ),
            make_action("sys.info", "Inspects system properties", "system"),
            make_action("fs.read_file", "Reads file contents", "filesystem"),
        ),
        make_item(
            "Calculate square root of 256",
            "calculator",
            make_action(
                "calculator",
                "Evaluates math functions and arithmetic",
                "calculator",
            ),
            make_action("media.volume", "Sets volume level", "audio"),
            make_action(
                "display.resolution",
                "Queries display monitor resolution",
                "display",
            ),
        ),
        make_item(
            "Compute 2 raised to power of 10",
            "calculator",
            make_action(
                "calculator",
                "Evaluates exponential and arithmetic math",
                "calculator",
            ),
            make_action("clipboard.clear", "Clears clipboard contents", "clipboard"),
            make_action("process.list", "Lists running tasks", "process"),
        ),
        // 5. Audio
        make_item(
            "Set master volume to 50 percent",
            "media.volume",
            make_action(
                "media.volume",
                "Sets master audio playback volume level",
                "audio",
            ),
            make_action(
                "calculator",
                "Evaluates mathematical expressions",
                "calculator",
            ),
            make_action("process.kill", "Terminates a process", "process"),
        ),
        make_item(
            "Mute audio output sound",
            "media.mute",
            make_action("media.mute", "Mutes speaker sound output", "audio"),
            make_action("fs.delete_file", "Removes file from storage", "filesystem"),
            make_action("power.sleep", "Puts PC into sleep mode", "power"),
        ),
        make_item(
            "Unmute sound and restore audio",
            "media.unmute",
            make_action("media.unmute", "Restores unmuted audio playback", "audio"),
            make_action("network.ip", "Queries local network IP address", "network"),
            make_action("sys.diagnostic", "Checks system memory and CPU", "system"),
        ),
        // 6. Timer
        make_item(
            "Set a timer for 5 minutes",
            "timer.set",
            make_action(
                "timer.set",
                "Creates a countdown timer and reminder",
                "timer",
            ),
            make_action("calculator", "Solves math equations", "calculator"),
            make_action("fs.read_file", "Reads file text", "filesystem"),
        ),
        make_item(
            "Countdown 30 seconds for tea",
            "timer.set",
            make_action(
                "timer.set",
                "Sets a countdown timer for specified seconds",
                "timer",
            ),
            make_action("media.volume", "Sets audio volume", "audio"),
            make_action("power.restart", "Restarts operating system", "power"),
        ),
        // 7. Clipboard
        make_item(
            "Copy text to clipboard",
            "clipboard.set",
            make_action(
                "clipboard.set",
                "Copies text to system clipboard buffer",
                "clipboard",
            ),
            make_action("sys.info", "Inspects OS hardware", "system"),
            make_action("process.kill", "Terminates running process", "process"),
        ),
        make_item(
            "Paste text from clipboard",
            "clipboard.get",
            make_action(
                "clipboard.get",
                "Retrieves current text from clipboard buffer",
                "clipboard",
            ),
            make_action("calculator", "Calculates numbers", "calculator"),
            make_action(
                "display.brightness",
                "Adjusts monitor brightness",
                "display",
            ),
        ),
        make_item(
            "Clear clipboard contents",
            "clipboard.clear",
            make_action(
                "clipboard.clear",
                "Empties system clipboard buffer",
                "clipboard",
            ),
            make_action("network.ping", "Pings network server", "network"),
            make_action("media.volume", "Sets audio volume", "audio"),
        ),
        // 8. Network
        make_item(
            "Ping google.com to check connectivity",
            "network.ping",
            make_action(
                "network.ping",
                "Sends ICMP echo request packets to verify network connection",
                "network",
            ),
            make_action("calculator", "Solves arithmetic calculations", "calculator"),
            make_action("fs.write_file", "Writes text to disk file", "filesystem"),
        ),
        make_item(
            "What is my local IP address",
            "network.ip",
            make_action(
                "network.ip",
                "Retrieves current network interface IP addresses",
                "network",
            ),
            make_action("power.sleep", "Puts computer to sleep", "power"),
            make_action("media.mute", "Mutes audio playback", "audio"),
        ),
        make_item(
            "Show active listening network connections",
            "network.connections",
            make_action(
                "network.connections",
                "Lists open TCP/UDP sockets and ports",
                "network",
            ),
            make_action("display.resolution", "Checks screen dimensions", "display"),
            make_action("timer.set", "Sets a countdown reminder", "timer"),
        ),
        // 9. Power
        make_item(
            "Put computer into sleep mode",
            "power.sleep",
            make_action(
                "power.sleep",
                "Suspends computer into power-saving sleep state",
                "power",
            ),
            make_action("calculator", "Computes arithmetic numbers", "calculator"),
            make_action("fs.read_file", "Reads document text", "filesystem"),
        ),
        make_item(
            "Restart system reboot computer",
            "power.restart",
            make_action(
                "power.restart",
                "Reboots the operating system cleanly",
                "power",
            ),
            make_action("clipboard.get", "Gets clipboard text", "clipboard"),
            make_action("media.volume", "Sets volume level", "audio"),
        ),
        make_item(
            "Lock screen workstation immediately",
            "power.lock",
            make_action(
                "power.lock",
                "Locks the active user session screen",
                "power",
            ),
            make_action("process.list", "Lists running tasks", "process"),
            make_action("sys.info", "Shows OS version", "system"),
        ),
        // 10. Display
        make_item(
            "Check current screen resolution and refresh rate",
            "display.resolution",
            make_action(
                "display.resolution",
                "Inspects monitor display resolution and DPI",
                "display",
            ),
            make_action("media.volume", "Changes audio loudness", "audio"),
            make_action("calculator", "Calculates expressions", "calculator"),
        ),
        make_item(
            "Adjust monitor brightness to 80 percent",
            "display.brightness",
            make_action(
                "display.brightness",
                "Sets screen display brightness level",
                "display",
            ),
            make_action("network.ping", "Tests host reachability", "network"),
            make_action("fs.delete_file", "Deletes file", "filesystem"),
        ),
        // 11. Services
        make_item(
            "Check status of docker system service",
            "services.status",
            make_action(
                "services.status",
                "Queries background system daemon service state",
                "services",
            ),
            make_action("clipboard.clear", "Clears clipboard text", "clipboard"),
            make_action("media.mute", "Mutes speaker sound", "audio"),
        ),
        make_item(
            "Restart nginx web server service",
            "services.restart",
            make_action(
                "services.restart",
                "Restarts a background system service",
                "services",
            ),
            make_action("power.sleep", "Suspends workstation", "power"),
            make_action("calculator", "Performs math evaluation", "calculator"),
        ),
        // 12. Registry & Config
        make_item(
            "Query windows registry setting key",
            "registry.query",
            make_action(
                "registry.query",
                "Reads configuration keys from system registry",
                "registry_config",
            ),
            make_action("media.volume", "Changes speaker volume", "audio"),
            make_action("fs.list_directory", "Lists directory files", "filesystem"),
        ),
        make_item(
            "Read application configuration values",
            "config.read",
            make_action(
                "config.read",
                "Reads persistent system configuration parameters",
                "registry_config",
            ),
            make_action("timer.set", "Sets countdown timer", "timer"),
            make_action("process.kill", "Terminates task", "process"),
        ),
        // 13. Disk & Storage
        make_item(
            "Check free disk space on drive",
            "disk.free_space",
            make_action(
                "disk.free_space",
                "Reports available free capacity across storage drives",
                "disk_storage",
            ),
            make_action("calculator", "Calculates numbers", "calculator"),
            make_action("media.mute", "Mutes audio playback", "audio"),
        ),
        make_item(
            "List all mounted storage disk volumes",
            "disk.list_volumes",
            make_action(
                "disk.list_volumes",
                "Enumerates mounted filesystem partition volumes",
                "disk_storage",
            ),
            make_action("clipboard.get", "Retrieves clipboard text", "clipboard"),
            make_action("power.lock", "Locks desktop screen", "power"),
        ),
        // 14. Firewall
        make_item(
            "Check system firewall protection status",
            "firewall.status",
            make_action(
                "firewall.status",
                "Inspects status of network firewall filtering rules",
                "firewall",
            ),
            make_action("fs.read_file", "Reads file contents", "filesystem"),
            make_action("display.brightness", "Adjusts brightness", "display"),
        ),
        make_item(
            "Block incoming network port 8080",
            "firewall.block",
            make_action(
                "firewall.block",
                "Adds firewall rule blocking port traffic",
                "firewall",
            ),
            make_action("media.volume", "Sets volume level", "audio"),
            make_action("calculator", "Evaluates formula", "calculator"),
        ),
        // 15. Users & Auth
        make_item(
            "Who is the current logged in user account",
            "user.current",
            make_action(
                "user.current",
                "Reports active username and authentication privileges",
                "users_auth",
            ),
            make_action("power.sleep", "Suspends PC", "power"),
            make_action("timer.set", "Sets a timer", "timer"),
        ),
        make_item(
            "List all registered user accounts on system",
            "user.list",
            make_action(
                "user.list",
                "Lists local user accounts and groups",
                "users_auth",
            ),
            make_action("clipboard.set", "Copies text to clipboard", "clipboard"),
            make_action("media.unmute", "Restores sound output", "audio"),
        ),
        // 16. Devices & Hardware
        make_item(
            "List connected USB hardware devices",
            "devices.usb",
            make_action(
                "devices.usb",
                "Enumerates connected USB controllers and peripherals",
                "devices_hardware",
            ),
            make_action("calculator", "Evaluates arithmetic math", "calculator"),
            make_action("fs.write_file", "Writes file", "filesystem"),
        ),
        make_item(
            "Inspect PCI bus hardware adapters",
            "devices.pci",
            make_action(
                "devices.pci",
                "Lists installed PCI and PCIe expansion cards",
                "devices_hardware",
            ),
            make_action("media.volume", "Sets volume", "audio"),
            make_action("power.restart", "Reboots computer", "power"),
        ),
        // 17. Package Management
        make_item(
            "List installed software packages and applications",
            "package.list",
            make_action(
                "package.list",
                "Queries installed system packages and software",
                "package_mgmt",
            ),
            make_action("network.ip", "Queries IP address", "network"),
            make_action("clipboard.clear", "Clears clipboard", "clipboard"),
        ),
        make_item(
            "Check for available system software updates",
            "package.check_updates",
            make_action(
                "package.check_updates",
                "Checks repository for pending package upgrades",
                "package_mgmt",
            ),
            make_action("display.resolution", "Checks screen dimensions", "display"),
            make_action("timer.set", "Sets reminder timer", "timer"),
        ),
        // 18. Logs & Events
        make_item(
            "Inspect system error event logs",
            "logs.system_errors",
            make_action(
                "logs.system_errors",
                "Reads operating system error logs and diagnostic events",
                "logs_events",
            ),
            make_action("calculator", "Calculates math", "calculator"),
            make_action("media.mute", "Mutes volume", "audio"),
        ),
        make_item(
            "View systemd journal log entries",
            "logs.journal",
            make_action(
                "logs.journal",
                "Reads system log journal entries",
                "logs_events",
            ),
            make_action("power.sleep", "Suspends PC", "power"),
            make_action("fs.delete_file", "Deletes file", "filesystem"),
        ),
        // 19. Scheduled Tasks
        make_item(
            "List all scheduled cron jobs and tasks",
            "tasks.list_scheduled",
            make_action(
                "tasks.list_scheduled",
                "Enumerates scheduled background automated tasks",
                "scheduled_tasks",
            ),
            make_action("media.volume", "Sets volume level", "audio"),
            make_action("sys.info", "Inspects OS version", "system"),
        ),
        make_item(
            "Create new scheduled background task timer",
            "tasks.create_cron",
            make_action(
                "tasks.create_cron",
                "Configures recurring scheduled timer execution",
                "scheduled_tasks",
            ),
            make_action("clipboard.get", "Gets clipboard text", "clipboard"),
            make_action("calculator", "Solves math expressions", "calculator"),
        ),
        // 20. Notifications
        make_item(
            "Send desktop notification popup alert",
            "notification.send",
            make_action(
                "notification.send",
                "Dispatches visual toast notification message",
                "notifications",
            ),
            make_action("power.restart", "Restarts operating system", "power"),
            make_action("network.ping", "Pings host server", "network"),
        ),
        make_item(
            "Dismiss all active desktop notification toasts",
            "notification.dismiss",
            make_action(
                "notification.dismiss",
                "Clears notification center banner popups",
                "notifications",
            ),
            make_action("fs.read_file", "Reads file contents", "filesystem"),
            make_action("media.volume", "Adjusts volume", "audio"),
        ),
        // 21. Window Management
        make_item(
            "Focus active visual studio code window",
            "window.focus",
            make_action(
                "window.focus",
                "Brings target application window into foreground focus",
                "window_mgmt",
            ),
            make_action("calculator", "Calculates equations", "calculator"),
            make_action("timer.set", "Sets countdown timer", "timer"),
        ),
        make_item(
            "Minimize all open desktop application windows",
            "window.minimize_all",
            make_action(
                "window.minimize_all",
                "Minimizes windows showing clear desktop",
                "window_mgmt",
            ),
            make_action("display.brightness", "Adjusts brightness", "display"),
            make_action("network.ip", "Queries network IP", "network"),
        ),
        // 22. Bluetooth
        make_item(
            "Turn on bluetooth wireless adapter",
            "bluetooth.enable",
            make_action(
                "bluetooth.enable",
                "Powers on local Bluetooth radio hardware",
                "bluetooth",
            ),
            make_action("fs.write_file", "Writes text file", "filesystem"),
            make_action("clipboard.clear", "Clears clipboard text", "clipboard"),
        ),
        make_item(
            "List paired bluetooth devices and headphones",
            "bluetooth.paired",
            make_action(
                "bluetooth.paired",
                "Enumerates known paired Bluetooth devices",
                "bluetooth",
            ),
            make_action("media.volume", "Sets audio level", "audio"),
            make_action("sys.info", "Inspects OS version", "system"),
        ),
    ]
}
