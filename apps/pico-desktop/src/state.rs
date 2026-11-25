use chrono::{DateTime, Duration, Local, Utc};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ActiveTab {
    Media,
    Timers,
    Notes,
    Stash,
    Ask,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TimerEntry {
    pub id: String,
    pub label: String,
    pub duration_secs: u64,
    pub remaining_secs: u64,
    pub paused: bool,
    pub deadline_utc: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PomodoroPhase {
    Work,
    ShortBreak,
    LongBreak,
}

impl PomodoroPhase {
    pub fn label(self) -> &'static str {
        match self {
            PomodoroPhase::Work => "Focus",
            PomodoroPhase::ShortBreak => "Short Break",
            PomodoroPhase::LongBreak => "Long Break",
        }
    }

    pub fn default_secs(self) -> u64 {
        match self {
            PomodoroPhase::Work => 25 * 60,
            PomodoroPhase::ShortBreak => 5 * 60,
            PomodoroPhase::LongBreak => 15 * 60,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PomodoroState {
    pub phase: PomodoroPhase,
    pub remaining_secs: u64,
    pub running: bool,
    pub deadline_utc: Option<DateTime<Utc>>,
    pub auto_advance: bool,
    pub completed_today: u32,
    pub completed_date_ymd: String,
}

impl Default for PomodoroState {
    fn default() -> Self {
        Self {
            phase: PomodoroPhase::Work,
            remaining_secs: PomodoroPhase::Work.default_secs(),
            running: false,
            deadline_utc: None,
            auto_advance: false,
            completed_today: 0,
            completed_date_ymd: Local::now().format("%Y-%m-%d").to_string(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteEntry {
    pub id: String,
    pub text: String,
    pub pinned: bool,
    pub tag: String,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum StashKind {
    File,
    Url,
    Text,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StashEntry {
    pub id: String,
    pub kind: StashKind,
    pub label: String,
    pub payload: String,
    pub missing_on_disk: bool,
    #[serde(default)]
    pub size_bytes: Option<u64>,
    #[serde(default)]
    pub is_dir: bool,
    pub added_at: DateTime<Utc>,
}

impl StashEntry {
    /// Returns a concise human-readable preview line for the companion UI.
    pub fn preview(&self) -> String {
        match self.kind {
            StashKind::File => {
                if self.missing_on_disk {
                    "Missing on disk".into()
                } else if self.is_dir {
                    "Directory".into()
                } else if let Some(bytes) = self.size_bytes {
                    if bytes < 1024 {
                        format!("{} B", bytes)
                    } else if bytes < 1024 * 1024 {
                        format!("{:.1} KB", bytes as f64 / 1024.0)
                    } else {
                        format!("{:.1} MB", bytes as f64 / (1024.0 * 1024.0))
                    }
                } else {
                    "File".into()
                }
            }
            StashKind::Url => self
                .payload
                .trim_start_matches("https://")
                .trim_start_matches("http://")
                .chars()
                .take(44)
                .collect(),
            StashKind::Text => {
                let first_line = self.payload.lines().next().unwrap_or("").trim();
                let chars = self.payload.chars().count();
                let snippet: String = first_line.chars().take(28).collect();
                format!("{} chars · {}", chars, snippet)
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StashAddOutcome {
    pub id: String,
    pub duplicate: bool,
    pub truncated: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompanionNotification {
    pub title: String,
    pub body: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CompanionState {
    pub active_tab: ActiveTab,
    pub compact_mode: bool,
    pub timers: Vec<TimerEntry>,
    pub pomodoro: PomodoroState,
    pub notes: Vec<NoteEntry>,
    pub stash: Vec<StashEntry>,
    #[serde(default)]
    next_seq: u64,
}

impl Default for CompanionState {
    fn default() -> Self {
        Self {
            active_tab: ActiveTab::Media,
            compact_mode: true,
            timers: Vec::new(),
            pomodoro: PomodoroState::default(),
            notes: Vec::new(),
            stash: Vec::new(),
            next_seq: 1,
        }
    }
}

impl CompanionState {
    pub fn default_storage_path() -> PathBuf {
        #[cfg(target_os = "windows")]
        {
            if let Ok(local_app) = std::env::var("LOCALAPPDATA") {
                return PathBuf::from(local_app)
                    .join("Pico")
                    .join("companion_state.json");
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            if let Ok(xdg_data) = std::env::var("XDG_DATA_HOME") {
                let t = xdg_data.trim();
                if !t.is_empty() {
                    return PathBuf::from(t).join("pico").join("companion_state.json");
                }
            }
            if let Ok(home) = std::env::var("HOME") {
                return PathBuf::from(home)
                    .join(".local/share/pico")
                    .join("companion_state.json");
            }
        }
        PathBuf::from("data/companion_state.json")
    }

    pub fn load_or_default(path: &Path) -> Self {
        let Ok(raw) = std::fs::read_to_string(path) else {
            return Self::default();
        };
        let mut loaded: Self = serde_json::from_str(&raw).unwrap_or_default();
        loaded.reconcile_clock(Utc::now(), &Local::now().format("%Y-%m-%d").to_string());
        loaded.refresh_stash_existence();
        loaded
    }

    /// Saves state atomically via write-to-temp + rename so sudden exit never corrupts user data.
    pub fn save_atomic(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)?;
            }
        }
        let tmp = path.with_extension("json.tmp");
        let payload = serde_json::to_vec_pretty(self)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        std::fs::write(&tmp, payload)?;
        if std::fs::rename(&tmp, path).is_err() {
            let _ = std::fs::remove_file(path);
            std::fs::rename(&tmp, path)?;
        }
        Ok(())
    }

    fn alloc_id(&mut self, prefix: &str) -> String {
        self.next_seq = self.next_seq.wrapping_add(1).max(1);
        format!("{}-{}", prefix, self.next_seq)
    }

    pub fn has_running_clocks(&self) -> bool {
        self.pomodoro.running
            || self
                .timers
                .iter()
                .any(|t| !t.paused && t.remaining_secs > 0)
    }

    /// Reconciles active wall-clock deadlines and returns notifications for any completed clocks.
    pub fn reconcile_clock(
        &mut self,
        now_utc: DateTime<Utc>,
        today_ymd: &str,
    ) -> Vec<CompanionNotification> {
        let mut notifications = Vec::new();

        if self.pomodoro.completed_date_ymd != today_ymd {
            self.pomodoro.completed_today = 0;
            self.pomodoro.completed_date_ymd = today_ymd.to_string();
        }

        if self.pomodoro.running {
            if let Some(deadline) = self.pomodoro.deadline_utc {
                let rem = (deadline - now_utc).num_seconds();
                if rem <= 0 {
                    let finished_phase = self.pomodoro.phase;
                    if finished_phase == PomodoroPhase::Work {
                        self.pomodoro.completed_today += 1;
                    }
                    let next = match finished_phase {
                        PomodoroPhase::Work => {
                            if self.pomodoro.completed_today.is_multiple_of(4) {
                                PomodoroPhase::LongBreak
                            } else {
                                PomodoroPhase::ShortBreak
                            }
                        }
                        PomodoroPhase::ShortBreak | PomodoroPhase::LongBreak => PomodoroPhase::Work,
                    };
                    notifications.push(CompanionNotification {
                        title: format!("Pomodoro {} Complete", finished_phase.label()),
                        body: format!("Ready for {}.", next.label()),
                    });
                    self.pomodoro.phase = next;
                    self.pomodoro.remaining_secs = next.default_secs();
                    if self.pomodoro.auto_advance {
                        self.pomodoro.running = true;
                        self.pomodoro.deadline_utc =
                            Some(now_utc + Duration::seconds(next.default_secs() as i64));
                    } else {
                        self.pomodoro.running = false;
                        self.pomodoro.deadline_utc = None;
                    }
                } else {
                    self.pomodoro.remaining_secs = rem as u64;
                }
            }
        }

        let mut remaining_timers = Vec::with_capacity(self.timers.len());
        for mut timer in std::mem::take(&mut self.timers) {
            if !timer.paused {
                if let Some(deadline) = timer.deadline_utc {
                    let rem = (deadline - now_utc).num_seconds();
                    if rem <= 0 {
                        notifications.push(CompanionNotification {
                            title: "Timer Elapsed".into(),
                            body: format!("'{}' finished ({}s).", timer.label, timer.duration_secs),
                        });
                        continue;
                    }
                    timer.remaining_secs = rem as u64;
                }
            }
            remaining_timers.push(timer);
        }
        self.timers = remaining_timers;

        notifications
    }

    pub fn add_timer(&mut self, label: &str, secs: u64) -> String {
        let secs = secs.max(1);
        let id = self.alloc_id("tmr");
        let now = Utc::now();
        let clean_label = if label.trim().is_empty() {
            format!("{}m Timer", (secs / 60).max(1))
        } else {
            label.trim().to_string()
        };
        self.timers.push(TimerEntry {
            id: id.clone(),
            label: clean_label,
            duration_secs: secs,
            remaining_secs: secs,
            paused: false,
            deadline_utc: Some(now + Duration::seconds(secs as i64)),
        });
        id
    }

    pub fn toggle_timer_pause(&mut self, id: &str) {
        let now = Utc::now();
        if let Some(t) = self.timers.iter_mut().find(|t| t.id == id) {
            if t.paused {
                t.paused = false;
                t.deadline_utc = Some(now + Duration::seconds(t.remaining_secs as i64));
            } else {
                if let Some(deadline) = t.deadline_utc {
                    t.remaining_secs = (deadline - now).num_seconds().max(0) as u64;
                }
                t.paused = true;
                t.deadline_utc = None;
            }
        }
    }

    pub fn reset_timer(&mut self, id: &str) {
        let now = Utc::now();
        if let Some(t) = self.timers.iter_mut().find(|t| t.id == id) {
            t.remaining_secs = t.duration_secs;
            if t.paused {
                t.deadline_utc = None;
            } else {
                t.deadline_utc = Some(now + Duration::seconds(t.duration_secs as i64));
            }
        }
    }

    pub fn remove_timer(&mut self, id: &str) {
        self.timers.retain(|t| t.id != id);
    }

    pub fn toggle_pomodoro(&mut self) {
        let now = Utc::now();
        if self.pomodoro.running {
            if let Some(deadline) = self.pomodoro.deadline_utc {
                self.pomodoro.remaining_secs = (deadline - now).num_seconds().max(0) as u64;
            }
            self.pomodoro.running = false;
            self.pomodoro.deadline_utc = None;
        } else {
            let rem = self.pomodoro.remaining_secs.max(1);
            self.pomodoro.running = true;
            self.pomodoro.deadline_utc = Some(now + Duration::seconds(rem as i64));
        }
    }

    pub fn set_pomodoro_phase(&mut self, phase: PomodoroPhase) {
        self.pomodoro.phase = phase;
        self.pomodoro.running = false;
        self.pomodoro.deadline_utc = None;
        self.pomodoro.remaining_secs = phase.default_secs();
    }

    pub fn add_note(&mut self, text: &str, tag: &str) -> Option<String> {
        let clean = text.trim();
        if clean.is_empty() {
            return None;
        }
        let id = self.alloc_id("note");
        self.notes.insert(
            0,
            NoteEntry {
                id: id.clone(),
                text: clean.to_string(),
                pinned: false,
                tag: tag.trim().to_string(),
                updated_at: Utc::now(),
            },
        );
        self.sort_notes();
        Some(id)
    }

    pub fn toggle_note_pin(&mut self, id: &str) {
        if let Some(n) = self.notes.iter_mut().find(|n| n.id == id) {
            n.pinned = !n.pinned;
            n.updated_at = Utc::now();
        }
        self.sort_notes();
    }

    pub fn edit_note(&mut self, id: &str, new_text: &str) {
        let clean = new_text.trim();
        if clean.is_empty() {
            return;
        }
        if let Some(n) = self.notes.iter_mut().find(|n| n.id == id) {
            n.text = clean.to_string();
            n.updated_at = Utc::now();
        }
    }

    pub fn remove_note(&mut self, id: &str) {
        self.notes.retain(|n| n.id != id);
    }

    fn sort_notes(&mut self) {
        self.notes.sort_by(|a, b| {
            b.pinned
                .cmp(&a.pinned)
                .then_with(|| b.updated_at.cmp(&a.updated_at))
        });
    }

    #[cfg(test)]
    pub fn add_stash_path(&mut self, path: &Path) -> String {
        self.add_stash_path_checked(path).id
    }

    pub fn add_stash_path_checked(&mut self, path: &Path) -> StashAddOutcome {
        let payload = path.to_string_lossy().into_owned();
        if let Some(pos) = self.stash.iter().position(|s| s.payload == payload) {
            let mut item = self.stash.remove(pos);
            item.missing_on_disk = !path.exists();
            let id = item.id.clone();
            self.stash.insert(0, item);
            return StashAddOutcome {
                id,
                duplicate: true,
                truncated: false,
            };
        }
        let label = path
            .file_name()
            .and_then(|s| s.to_str())
            .filter(|s| !s.is_empty())
            .unwrap_or(&payload)
            .to_string();
        let id = self.alloc_id("stash");
        let meta = std::fs::metadata(path).ok();
        let missing = meta.is_none() && !path.exists();
        let size_bytes = meta.as_ref().filter(|m| m.is_file()).map(|m| m.len());
        let is_dir = meta.as_ref().is_some_and(|m| m.is_dir());
        self.stash.insert(
            0,
            StashEntry {
                id: id.clone(),
                kind: StashKind::File,
                label,
                payload,
                missing_on_disk: missing,
                size_bytes,
                is_dir,
                added_at: Utc::now(),
            },
        );
        StashAddOutcome {
            id,
            duplicate: false,
            truncated: false,
        }
    }

    pub fn add_stash_text_or_url_checked(&mut self, raw: &str) -> Option<StashAddOutcome> {
        const MAX_STASH_CHARS: usize = 65_536;
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return None;
        }
        let mut truncated = false;
        let bounded: String = if trimmed.chars().count() > MAX_STASH_CHARS {
            truncated = true;
            trimmed.chars().take(MAX_STASH_CHARS).collect()
        } else {
            trimmed.to_string()
        };

        if let Some(pos) = self.stash.iter().position(|s| s.payload == bounded) {
            let item = self.stash.remove(pos);
            let id = item.id.clone();
            self.stash.insert(0, item);
            return Some(StashAddOutcome {
                id,
                duplicate: true,
                truncated,
            });
        }
        let kind = if bounded.starts_with("http://") || bounded.starts_with("https://") {
            StashKind::Url
        } else {
            let p = Path::new(&bounded);
            if p.exists() {
                return Some(self.add_stash_path_checked(p));
            }
            StashKind::Text
        };
        let label: String = bounded.chars().take(48).collect();
        let id = self.alloc_id("stash");
        self.stash.insert(
            0,
            StashEntry {
                id: id.clone(),
                kind,
                label,
                payload: bounded,
                missing_on_disk: false,
                size_bytes: None,
                is_dir: false,
                added_at: Utc::now(),
            },
        );
        Some(StashAddOutcome {
            id,
            duplicate: false,
            truncated,
        })
    }

    /// Unlinks an entry from the Stash view ONLY (never modifies or deletes underlying disk files).
    pub fn remove_stash(&mut self, id: &str) {
        self.stash.retain(|s| s.id != id);
    }

    pub fn refresh_stash_existence(&mut self) {
        for entry in &mut self.stash {
            if entry.kind == StashKind::File {
                entry.missing_on_disk = !Path::new(&entry.payload).exists();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_state_atomic_persistence_and_wall_clock_reconciliation() {
        let dir = std::env::temp_dir().join(format!(
            "pico_comp_test_{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        let path = dir.join("companion_state.json");

        let mut st = CompanionState::default();
        let t_id = st.add_timer("Tea", 120);
        st.add_note("Buy coffee", "personal");
        let n2 = st.add_note("Pinned task", "work").unwrap();
        st.toggle_note_pin(&n2);

        st.save_atomic(&path).unwrap();
        let mut loaded = CompanionState::load_or_default(&path);
        assert_eq!(loaded.timers.len(), 1);
        assert_eq!(loaded.notes.len(), 2);
        assert!(loaded.notes[0].pinned);

        loaded.toggle_timer_pause(&t_id);
        assert!(loaded.timers[0].paused);

        let future = Utc::now() + Duration::seconds(2000);
        let mut st_exp = CompanionState::default();
        st_exp.add_timer("Quick", 10);
        let notifs = st_exp.reconcile_clock(future, "2099-01-01");
        assert_eq!(notifs.len(), 1);
        assert!(st_exp.timers.is_empty());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_stash_remove_never_deletes_original_file_and_detects_missing() {
        let dir = std::env::temp_dir().join(format!(
            "pico_stash_test_{}",
            Utc::now().timestamp_nanos_opt().unwrap_or(0)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("keep_me.txt");
        std::fs::write(&file, "precious content").unwrap();

        let mut st = CompanionState::default();
        let sid = st.add_stash_path(&file);
        assert!(!st.stash[0].missing_on_disk);
        st.remove_stash(&sid);
        assert!(
            file.exists(),
            "Removing stash entry must never delete file on disk"
        );

        let _ = std::fs::remove_file(&file);
        let sid2 = st.add_stash_path(&file);
        assert!(
            st.stash
                .iter()
                .find(|s| s.id == sid2)
                .unwrap()
                .missing_on_disk
        );

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_compact_default_and_stash_dedup_and_truncation() {
        let mut st = CompanionState::default();
        assert!(
            st.compact_mode,
            "Compact mode must be the default launch state"
        );

        let out1 = st
            .add_stash_text_or_url_checked("https://example.com/pico")
            .unwrap();
        assert!(!out1.duplicate);
        assert_eq!(st.stash[0].kind, StashKind::Url);

        let _ = st.add_stash_text_or_url_checked("some snippet").unwrap();
        let dup = st
            .add_stash_text_or_url_checked("https://example.com/pico")
            .unwrap();
        assert!(dup.duplicate);
        assert_eq!(st.stash[0].id, out1.id);

        let huge = "a".repeat(70_000);
        let trunc = st.add_stash_text_or_url_checked(&huge).unwrap();
        assert!(trunc.truncated);
        assert!(st.stash[0].payload.chars().count() <= 65_536);
    }
}
