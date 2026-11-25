pub mod apps;
pub mod audio;
pub mod clipboard;
pub mod display;
pub mod sysinfo;

pub use apps::{PlatformApps, ProcessInfo};
pub use audio::{AudioEndpointState, PlatformAudio};
pub use clipboard::PlatformClipboard;
pub use display::PlatformDisplay;
pub use sysinfo::{PlatformSysInfo, SystemStats};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_system_stats_fetch() {
        let stats = PlatformSysInfo::get_system_stats();
        assert!(stats.is_ok());
        let s = stats.unwrap();
        assert!(!s.os.is_empty());
        assert!(s.cpu_cores > 0);
        assert!(s.total_memory_mb > 0);
    }

    #[test]
    fn test_list_processes() {
        let procs = PlatformApps::list_processes(5);
        assert!(procs.is_ok());
        let list = procs.unwrap();
        assert!(!list.is_empty());
    }
}
