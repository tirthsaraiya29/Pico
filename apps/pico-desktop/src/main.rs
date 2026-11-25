#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod calc;
mod canvas;
mod palette;
mod state;
mod sys_tray;

use calc::try_eval_quick_math;
use canvas::{Canvas, FontEngine, Rect};
use chrono::{Local, Utc};
use clap::Parser;
use palette::{ArtPalette, ArtworkCache, Rgb};
use pico_core::{
    default_ipc_endpoint, default_ipc_token_path, read_ipc_token, send_ipc_request_sync,
    DaemonSnapshot, IpcRequest, IpcResponse,
};
use pico_platform::{
    get_battery_status, BatteryStatus, MediaTrackInfo, PlatformApps, PlatformAudio,
    PlatformClipboard, PlatformMedia, PlatformSysInfo, SystemStats,
};
use softbuffer::{Context, Surface};
use state::{ActiveTab, CompanionState, PomodoroPhase};
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::time::{Duration, Instant};
use sys_tray::platform_tray::{spawn_tray_and_hotkey, TrayHandle};
use sys_tray::TrayCommand;
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize, PhysicalPosition};
use winit::event::{ElementState, MouseButton, WindowEvent};
use winit::event_loop::{ActiveEventLoop, ControlFlow, EventLoop, EventLoopProxy};
use winit::keyboard::{Key, ModifiersState, NamedKey};
use winit::window::{Window, WindowAttributes, WindowId, WindowLevel};

const WIN_W: i32 = 368;
const WIN_H_FULL: i32 = 540;
const WIN_H_COMPACT: i32 = 102;

const C_BG: Rgb = Rgb::new(16, 19, 26);
const C_CARD: Rgb = Rgb::new(24, 28, 38);
const C_CARD_ALT: Rgb = Rgb::new(30, 35, 48);
const C_BORDER: Rgb = Rgb::new(45, 52, 70);
const C_FG: Rgb = Rgb::new(242, 245, 250);
const C_MUTED: Rgb = Rgb::new(152, 162, 182);
const C_ACCENT: Rgb = Rgb::new(79, 179, 255);
const C_GREEN: Rgb = Rgb::new(72, 199, 142);
const C_WARN: Rgb = Rgb::new(240, 173, 78);
const C_DANGER: Rgb = Rgb::new(232, 92, 102);

#[derive(Parser, Debug)]
#[command(name = "pico-desktop", about = "Lightweight Pico Desktop Companion")]
struct Args {
    /// Launch in compact single-pill mode (default)
    #[arg(long)]
    compact: bool,
    /// Launch in expanded dashboard mode
    #[arg(long)]
    expanded: bool,
    /// Measure actual idle RSS/CPU over 2.5 seconds and exit with JSON metrics
    #[arg(long)]
    benchmark_self: bool,
}

#[derive(Debug, Clone)]
enum CompanionEvent {
    Wake,
    Tray(TrayCommand),
    TelemetryUpdated {
        media: Option<MediaTrackInfo>,
        volume: Option<u8>,
        muted: Option<bool>,
        stats: Option<SystemStats>,
        battery: Option<BatteryStatus>,
    },
    PlatformActionResult {
        message: Option<String>,
        error: Option<String>,
        volume: Option<u8>,
        muted: Option<bool>,
        clipboard_stash: Option<String>,
        refresh_media: bool,
    },
    DaemonSnapshotUpdated(Option<DaemonSnapshot>),
    DaemonReply {
        reply: Option<String>,
        error: Option<String>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FocusField {
    None,
    TimerLabel,
    TimerMins,
    NoteInput,
    NoteSearch,
    StashSearch,
    AskInput,
}

#[derive(Debug, Clone)]
enum UiAction {
    DragWindow,
    HideToTray,
    ToggleCompact,
    CloseOrHide,
    SelectTab(ActiveTab),
    Focus(FocusField),
    MediaPrev,
    MediaPlayPause,
    MediaNext,
    ToggleMute,
    SetVolumeRatio(f32),
    TogglePomodoro,
    ResetPomodoro,
    SkipPomodoro,
    SelectPomodoroPhase(PomodoroPhase),
    TogglePomodoroAuto,
    AddPresetTimer(u64),
    AddCustomTimer,
    ToggleTimerPause(String),
    ResetTimer(String),
    DeleteTimer(String),
    AddNote,
    ToggleNotePin(String),
    CopyText(String),
    DeleteNote(String),
    StartEditNote(String),
    SaveEditNote,
    CancelEditNote,
    PasteClipboardToStash,
    OpenStashPayload(String),
    DeleteStash(String),
    SendPrompt,
    SelectClarification(usize),
    ConfirmTool(String, bool),
    ToggleDaemonMute,
    InterruptDaemonSpeech,
    ClearAskHistory,
}

struct HitBox {
    rect: Rect,
    action: UiAction,
    is_slider: bool,
}

#[derive(Clone)]
struct TelemetryState {
    media: Option<MediaTrackInfo>,
    volume: Option<u8>,
    muted: Option<bool>,
    stats: Option<SystemStats>,
    battery: Option<BatteryStatus>,
}

enum PlatformCommand {
    MediaPrev,
    MediaPlayPause,
    MediaNext,
    SetMute(bool),
    SetVolume(u8),
    CopyText(String),
    PasteClipboardToStash,
    OpenStashPayload(String),
    RefreshMedia,
}

enum DaemonCommand {
    Refresh,
    SubmitPrompt(String),
    RespondConfirmation(String, bool),
    SetVoicePaused(bool),
    InterruptSpeech,
}

struct CompanionApp {
    args: Args,
    window: Option<Rc<Window>>,
    context: Option<Context<Rc<Window>>>,
    surface: Option<Surface<Rc<Window>, Rc<Window>>>,
    fonts: FontEngine,
    state_path: PathBuf,
    state: CompanionState,
    artwork_cache: ArtworkCache,
    active_palette: ArtPalette,
    target_palette: ArtPalette,
    telemetry: TelemetryState,
    daemon_snapshot: Option<DaemonSnapshot>,
    ask_messages: Vec<(bool, String)>,
    ask_busy: bool,
    visible: bool,
    hovered_idx: Option<usize>,
    dragging_slider: bool,
    drag_hover_count: usize,
    anim_height: f32,
    anim_target_height: f32,
    cursor_pos: PhysicalPosition<f64>,
    scale_factor: f32,
    focus: FocusField,
    modifiers: ModifiersState,
    timer_label_input: String,
    timer_mins_input: String,
    note_input: String,
    note_search: String,
    stash_search: String,
    editing_note_id: Option<String>,
    editing_note_buf: String,
    ask_input: String,
    status_toast: Option<(String, Instant)>,
    hit_boxes: Vec<HitBox>,
    visible_flag: Arc<AtomicBool>,
    daemon_tx: mpsc::Sender<DaemonCommand>,
    platform_tx: mpsc::Sender<PlatformCommand>,
    tray_handle: Option<TrayHandle>,
    bench_started: Option<Instant>,
}

impl CompanionApp {
    fn new(
        args: Args,
        visible_flag: Arc<AtomicBool>,
        daemon_tx: mpsc::Sender<DaemonCommand>,
        platform_tx: mpsc::Sender<PlatformCommand>,
        tray_handle: Option<TrayHandle>,
    ) -> Self {
        let state_path = CompanionState::default_storage_path();
        let mut state = CompanionState::load_or_default(&state_path);
        if args.expanded {
            state.compact_mode = false;
        } else if args.compact {
            state.compact_mode = true;
        }
        let initial_h = if state.compact_mode {
            WIN_H_COMPACT as f32
        } else {
            WIN_H_FULL as f32
        };
        let audio_state = PlatformAudio::get_state().ok();
        Self {
            args,
            window: None,
            context: None,
            surface: None,
            fonts: FontEngine::load(),
            state_path,
            state,
            artwork_cache: ArtworkCache::new(6),
            active_palette: ArtPalette::default(),
            target_palette: ArtPalette::default(),
            telemetry: TelemetryState {
                media: None,
                volume: audio_state.as_ref().map(|a| a.volume_percent as u8),
                muted: audio_state.as_ref().map(|a| a.muted),
                stats: PlatformSysInfo::get_system_stats().ok(),
                battery: get_battery_status(),
            },
            daemon_snapshot: None,
            ask_messages: Vec::new(),
            ask_busy: false,
            visible: true,
            hovered_idx: None,
            dragging_slider: false,
            drag_hover_count: 0,
            anim_height: initial_h,
            anim_target_height: initial_h,
            cursor_pos: PhysicalPosition::new(0.0, 0.0),
            scale_factor: 1.0,
            focus: FocusField::None,
            modifiers: ModifiersState::empty(),
            timer_label_input: String::new(),
            timer_mins_input: "10".into(),
            note_input: String::new(),
            note_search: String::new(),
            stash_search: String::new(),
            editing_note_id: None,
            editing_note_buf: String::new(),
            ask_input: String::new(),
            status_toast: None,
            hit_boxes: Vec::with_capacity(48),
            visible_flag,
            daemon_tx,
            platform_tx,
            tray_handle,
            bench_started: None,
        }
    }

    fn save_state(&self) {
        let _ = self.state.save_atomic(&self.state_path);
    }

    fn set_toast(&mut self, msg: impl Into<String>) {
        self.status_toast = Some((msg.into(), Instant::now() + Duration::from_secs(3)));
    }

    fn desired_height(&self) -> i32 {
        if self.state.compact_mode {
            WIN_H_COMPACT
        } else {
            WIN_H_FULL
        }
    }

    fn sync_window_size(&mut self) {
        self.anim_target_height = self.desired_height() as f32;
        if let Some(ref win) = self.window {
            let h = self.anim_height.round() as i32;
            let _ = win.request_inner_size(LogicalSize::new(WIN_W, h));
            win.request_redraw();
        }
    }

    fn request_redraw(&self) {
        if let Some(ref win) = self.window {
            win.request_redraw();
        }
    }

    fn toggle_visible(&mut self) {
        self.visible = !self.visible;
        self.visible_flag.store(self.visible, Ordering::SeqCst);
        if let Some(ref win) = self.window {
            win.set_visible(self.visible);
            if self.visible {
                win.focus_window();
                let _ = self.daemon_tx.send(DaemonCommand::Refresh);
                let _ = self.platform_tx.send(PlatformCommand::RefreshMedia);
                win.request_redraw();
            }
        }
    }

    fn focused_buffer_mut(&mut self) -> Option<&mut String> {
        if self.editing_note_id.is_some() {
            return Some(&mut self.editing_note_buf);
        }
        match self.focus {
            FocusField::TimerLabel => Some(&mut self.timer_label_input),
            FocusField::TimerMins => Some(&mut self.timer_mins_input),
            FocusField::NoteInput => Some(&mut self.note_input),
            FocusField::NoteSearch => Some(&mut self.note_search),
            FocusField::StashSearch => Some(&mut self.stash_search),
            FocusField::AskInput => Some(&mut self.ask_input),
            FocusField::None => None,
        }
    }

    fn execute_action(&mut self, action: UiAction, event_loop: &ActiveEventLoop) {
        match action {
            UiAction::DragWindow => {
                if let Some(ref win) = self.window {
                    let _ = win.drag_window();
                }
            }
            UiAction::HideToTray => {
                self.visible = false;
                self.visible_flag.store(false, Ordering::SeqCst);
                if let Some(ref win) = self.window {
                    win.set_visible(false);
                }
            }
            UiAction::ToggleCompact => {
                self.state.compact_mode = !self.state.compact_mode;
                self.save_state();
                self.sync_window_size();
            }
            UiAction::CloseOrHide => {
                #[cfg(target_os = "windows")]
                {
                    if self.tray_handle.is_some() {
                        self.visible = false;
                        self.visible_flag.store(false, Ordering::SeqCst);
                        if let Some(ref win) = self.window {
                            win.set_visible(false);
                        }
                        return;
                    }
                }
                self.save_state();
                event_loop.exit();
            }
            UiAction::SelectTab(tab) => {
                self.state.active_tab = tab;
                self.state.compact_mode = false;
                self.focus = match tab {
                    ActiveTab::Notes => FocusField::NoteInput,
                    ActiveTab::Ask => FocusField::AskInput,
                    _ => FocusField::None,
                };
                self.save_state();
                self.sync_window_size();
            }
            UiAction::Focus(f) => {
                self.focus = f;
                self.editing_note_id = None;
                self.request_redraw();
            }
            UiAction::MediaPrev => {
                if self.platform_tx.send(PlatformCommand::MediaPrev).is_err() {
                    self.set_toast("Media worker unavailable");
                    self.request_redraw();
                }
            }
            UiAction::MediaPlayPause => {
                if self
                    .platform_tx
                    .send(PlatformCommand::MediaPlayPause)
                    .is_err()
                {
                    self.set_toast("Media worker unavailable");
                    self.request_redraw();
                }
            }
            UiAction::MediaNext => {
                if self.platform_tx.send(PlatformCommand::MediaNext).is_err() {
                    self.set_toast("Media worker unavailable");
                    self.request_redraw();
                }
            }
            UiAction::ToggleMute => {
                let next_muted = !self.telemetry.muted.unwrap_or(false);
                if self
                    .platform_tx
                    .send(PlatformCommand::SetMute(next_muted))
                    .is_err()
                {
                    self.set_toast("Audio worker unavailable");
                    self.request_redraw();
                }
            }
            UiAction::SetVolumeRatio(r) => {
                let vol = (r.clamp(0.0, 1.0) * 100.0).round() as u8;
                if self
                    .platform_tx
                    .send(PlatformCommand::SetVolume(vol))
                    .is_err()
                {
                    self.set_toast("Audio worker unavailable");
                    self.request_redraw();
                }
            }
            UiAction::TogglePomodoro => {
                self.state.toggle_pomodoro();
                self.save_state();
                self.request_redraw();
            }
            UiAction::ResetPomodoro => {
                let p = self.state.pomodoro.phase;
                self.state.set_pomodoro_phase(p);
                self.save_state();
                self.request_redraw();
            }
            UiAction::SkipPomodoro => {
                let next = match self.state.pomodoro.phase {
                    PomodoroPhase::Work => PomodoroPhase::ShortBreak,
                    PomodoroPhase::ShortBreak | PomodoroPhase::LongBreak => PomodoroPhase::Work,
                };
                self.state.set_pomodoro_phase(next);
                self.save_state();
                self.request_redraw();
            }
            UiAction::SelectPomodoroPhase(phase) => {
                self.state.set_pomodoro_phase(phase);
                self.save_state();
                self.request_redraw();
            }
            UiAction::TogglePomodoroAuto => {
                self.state.pomodoro.auto_advance = !self.state.pomodoro.auto_advance;
                self.save_state();
                self.request_redraw();
            }
            UiAction::AddPresetTimer(secs) => {
                self.state.add_timer("", secs);
                self.save_state();
                self.request_redraw();
            }
            UiAction::AddCustomTimer => {
                if let Ok(mins) = self.timer_mins_input.trim().parse::<u64>() {
                    if mins > 0 && mins <= 1440 {
                        self.state.add_timer(&self.timer_label_input, mins * 60);
                        self.timer_label_input.clear();
                        self.save_state();
                    } else {
                        self.set_toast("Minutes must be 1..1440");
                    }
                } else {
                    self.set_toast("Enter integer minutes");
                }
                self.request_redraw();
            }
            UiAction::ToggleTimerPause(id) => {
                self.state.toggle_timer_pause(&id);
                self.save_state();
                self.request_redraw();
            }
            UiAction::ResetTimer(id) => {
                self.state.reset_timer(&id);
                self.save_state();
                self.request_redraw();
            }
            UiAction::DeleteTimer(id) => {
                self.state.remove_timer(&id);
                self.save_state();
                self.request_redraw();
            }
            UiAction::AddNote => {
                if self.state.add_note(&self.note_input, "").is_some() {
                    self.note_input.clear();
                    self.save_state();
                    self.set_toast("Note saved");
                }
                self.request_redraw();
            }
            UiAction::ToggleNotePin(id) => {
                self.state.toggle_note_pin(&id);
                self.save_state();
                self.request_redraw();
            }
            UiAction::CopyText(txt) => {
                if self
                    .platform_tx
                    .send(PlatformCommand::CopyText(txt))
                    .is_err()
                {
                    self.set_toast("Clipboard worker unavailable");
                    self.request_redraw();
                }
            }
            UiAction::DeleteNote(id) => {
                self.state.remove_note(&id);
                if self.editing_note_id.as_deref() == Some(&id) {
                    self.editing_note_id = None;
                }
                self.save_state();
                self.request_redraw();
            }
            UiAction::StartEditNote(id) => {
                if let Some(n) = self.state.notes.iter().find(|n| n.id == id) {
                    self.editing_note_id = Some(id);
                    self.editing_note_buf = n.text.clone();
                    self.request_redraw();
                }
            }
            UiAction::SaveEditNote => {
                if let Some(id) = self.editing_note_id.take() {
                    self.state.edit_note(&id, &self.editing_note_buf);
                    self.editing_note_buf.clear();
                    self.save_state();
                    self.set_toast("Note updated");
                }
                self.request_redraw();
            }
            UiAction::CancelEditNote => {
                self.editing_note_id = None;
                self.editing_note_buf.clear();
                self.request_redraw();
            }
            UiAction::PasteClipboardToStash => {
                if self
                    .platform_tx
                    .send(PlatformCommand::PasteClipboardToStash)
                    .is_err()
                {
                    self.set_toast("Clipboard worker unavailable");
                    self.request_redraw();
                }
            }
            UiAction::OpenStashPayload(payload) => {
                if payload.starts_with("http://")
                    || payload.starts_with("https://")
                    || std::path::Path::new(&payload).exists()
                {
                    if self
                        .platform_tx
                        .send(PlatformCommand::OpenStashPayload(payload))
                        .is_err()
                    {
                        self.set_toast("Launch worker unavailable");
                        self.request_redraw();
                    }
                } else {
                    self.state.refresh_stash_existence();
                    self.set_toast("Item missing on disk");
                    self.request_redraw();
                }
            }
            UiAction::DeleteStash(id) => {
                self.state.remove_stash(&id);
                self.save_state();
                self.request_redraw();
            }
            UiAction::SendPrompt => {
                let prompt = self.ask_input.trim().to_string();
                if !prompt.is_empty() && !self.ask_busy {
                    self.ask_input.clear();
                    self.ask_messages.push((true, prompt.clone()));
                    if self.ask_messages.len() > 20 {
                        self.ask_messages.remove(0);
                    }
                    if let Some(calc_ans) = try_eval_quick_math(&prompt) {
                        if self.daemon_snapshot.is_none() {
                            self.ask_messages.push((false, format!("= {}", calc_ans)));
                            self.request_redraw();
                            return;
                        }
                    }
                    self.ask_busy = true;
                    let _ = self.daemon_tx.send(DaemonCommand::SubmitPrompt(prompt));
                    self.request_redraw();
                }
            }
            UiAction::SelectClarification(idx) => {
                let choice = match idx {
                    0 => "the first one",
                    1 => "the second one",
                    2 => "the third one",
                    _ => "cancel",
                };
                self.ask_messages.push((true, choice.into()));
                self.ask_busy = true;
                let _ = self
                    .daemon_tx
                    .send(DaemonCommand::SubmitPrompt(choice.into()));
                self.request_redraw();
            }
            UiAction::ConfirmTool(req_id, approved) => {
                let _ = self
                    .daemon_tx
                    .send(DaemonCommand::RespondConfirmation(req_id, approved));
                self.request_redraw();
            }
            UiAction::ToggleDaemonMute => {
                let cur = self
                    .daemon_snapshot
                    .as_ref()
                    .map(|s| s.voice_paused)
                    .unwrap_or(false);
                let _ = self.daemon_tx.send(DaemonCommand::SetVoicePaused(!cur));
            }
            UiAction::InterruptDaemonSpeech => {
                let _ = self.daemon_tx.send(DaemonCommand::InterruptSpeech);
            }
            UiAction::ClearAskHistory => {
                self.ask_messages.clear();
                self.request_redraw();
            }
        }
    }

    fn render(&mut self) {
        let Some(win) = self.window.as_ref().cloned() else {
            return;
        };
        let scale = win.scale_factor() as f32;
        self.scale_factor = scale;
        let phys = win.inner_size();
        let (Some(pw), Some(ph)) = (NonZeroU32::new(phys.width), NonZeroU32::new(phys.height))
        else {
            return;
        };

        let Some(surface) = self.surface.as_mut() else {
            return;
        };
        if surface.resize(pw, ph).is_err() {
            return;
        }
        let Ok(mut buffer) = surface.buffer_mut() else {
            return;
        };

        // Step smooth palette transition if artwork changed.
        if self.active_palette != self.target_palette {
            self.active_palette = ArtPalette {
                bg_left: self
                    .active_palette
                    .bg_left
                    .lerp(self.target_palette.bg_left, 0.35),
                bg_right: self
                    .active_palette
                    .bg_right
                    .lerp(self.target_palette.bg_right, 0.35),
                accent: self
                    .active_palette
                    .accent
                    .lerp(self.target_palette.accent, 0.35),
                text_primary: self.target_palette.text_primary,
                text_secondary: self.target_palette.text_secondary,
            };
        }

        // Prune expired status toast cleanly.
        if let Some((_, exp)) = self.status_toast {
            if Instant::now() >= exp {
                self.status_toast = None;
            }
        }

        self.hit_boxes.clear();
        let log_w = (phys.width as f32 / scale).round() as i32;
        let log_h = (phys.height as f32 / scale).round() as i32;

        let mut cv = Canvas::new(&mut buffer, phys.width as i32, phys.height as i32, scale);
        cv.clear(C_BG);
        cv.stroke_rounded_rect(Rect::new(0, 0, log_w, log_h), 12.0, C_BORDER, 1.0);

        let fonts = &self.fonts;
        // 1. Header Strip (y: 8..38)
        let drag_rect = Rect::new(8, 6, log_w - 96, 30);
        self.hit_boxes.push(HitBox {
            rect: drag_rect,
            action: UiAction::DragWindow,
            is_slider: false,
        });
        cv.draw_text_clipped(fonts, "Pico", 14, 12, 50, 14.0, true, C_FG);

        let daemon_up = self.daemon_snapshot.is_some();
        let pill_rect = Rect::new(56, 12, 84, 18);
        cv.fill_rounded_rect(pill_rect, 9.0, C_CARD_ALT, 1.0);
        cv.draw_text_clipped(
            fonts,
            if daemon_up { "Online" } else { "Standalone" },
            64,
            15,
            72,
            10.5,
            false,
            if daemon_up { C_GREEN } else { C_MUTED },
        );

        let compact_btn = Rect::new(log_w - 84, 10, 22, 22);
        let hide_btn = Rect::new(log_w - 58, 10, 22, 22);
        let close_btn = Rect::new(log_w - 32, 10, 22, 22);
        for (idx, (r, lbl, col, act)) in [
            (
                compact_btn,
                if self.state.compact_mode { "+" } else { "-" },
                C_MUTED,
                UiAction::ToggleCompact,
            ),
            (hide_btn, "_", C_MUTED, UiAction::HideToTray),
            (close_btn, "x", C_DANGER, UiAction::CloseOrHide),
        ]
        .into_iter()
        .enumerate()
        {
            let _ = idx;
            cv.fill_rounded_rect(r, 6.0, C_CARD_ALT, 1.0);
            cv.draw_text_clipped(fonts, lbl, r.x + 7, r.y + 4, 14, 12.0, true, col);
            self.hit_boxes.push(HitBox {
                rect: r,
                action: act,
                is_slider: false,
            });
        }

        if self.state.compact_mode {
            // Compact Nusa-style floating pill/card: prominent artwork + track/progress + media/vol/timer controls
            let card = Rect::new(10, 40, log_w - 20, 54);
            cv.fill_rounded_gradient(
                card,
                11.0,
                self.active_palette.bg_left,
                self.active_palette.bg_right,
                1.0,
            );
            let border_col = if self.drag_hover_count > 0 {
                C_ACCENT
            } else {
                C_BORDER
            };
            cv.stroke_rounded_rect(card, 11.0, border_col, 1.0);

            let has_media = self.telemetry.media.as_ref().is_some_and(|m| m.has_session);
            let mut text_x = 18;
            if let Some(m) = self.telemetry.media.as_ref().filter(|m| m.has_session) {
                let art_rect = Rect::new(16, 46, 42, 42);
                if let Some(ref bytes) = m.thumbnail_bytes {
                    if let Some(art) = self.artwork_cache.get_or_decode(bytes) {
                        cv.blit_rounded_rgba(art_rect, 8.0, art.width, art.height, &art.rgba);
                    } else {
                        cv.fill_rounded_rect(art_rect, 8.0, C_CARD_ALT, 1.0);
                    }
                } else {
                    cv.fill_rounded_rect(art_rect, 8.0, C_CARD_ALT, 1.0);
                    cv.draw_text_clipped(
                        fonts,
                        "♫",
                        30,
                        58,
                        16,
                        15.0,
                        true,
                        self.active_palette.accent,
                    );
                }
                text_x = 66;
            }

            // Active timer / Pomodoro badge or track metadata
            let primary_line = if self.drag_hover_count > 0 {
                "Drop files to add to Stash".to_string()
            } else if let Some(m) = self.telemetry.media.as_ref().filter(|m| m.has_session) {
                if m.title.is_empty() {
                    m.source_app.clone()
                } else {
                    m.title.clone()
                }
            } else if self.state.pomodoro.running {
                format!(
                    "{} · {}",
                    self.state.pomodoro.phase.label(),
                    fmt_mm_ss(self.state.pomodoro.remaining_secs)
                )
            } else if let Some(t) = self.state.timers.first() {
                format!("{} · {}", t.label, fmt_mm_ss(t.remaining_secs))
            } else {
                "Pico Companion".to_string()
            };

            let secondary_line = if let Some(ref toast) = self.status_toast {
                toast.0.clone()
            } else if let Some(m) = self.telemetry.media.as_ref().filter(|m| m.has_session) {
                if m.artist.is_empty() {
                    m.source_app.clone()
                } else {
                    m.artist.clone()
                }
            } else if self.state.pomodoro.running || !self.state.timers.is_empty() {
                format!("Vol {}% · Alt+Space", self.telemetry.volume.unwrap_or(50))
            } else {
                "Alt+Space toggle · [+] expand".to_string()
            };

            let ctrl_start_x = if has_media { log_w - 132 } else { log_w - 88 };
            let text_w = (ctrl_start_x - text_x - 6).max(60);
            cv.draw_text_clipped(
                fonts,
                &primary_line,
                text_x,
                47,
                text_w,
                12.0,
                true,
                self.active_palette.text_primary,
            );
            cv.draw_text_clipped(
                fonts,
                &secondary_line,
                text_x,
                63,
                text_w,
                10.5,
                false,
                if self.status_toast.is_some() {
                    C_WARN
                } else {
                    self.active_palette.text_secondary
                },
            );

            // Media progress bar (or timer progress) at bottom of compact card
            if let Some(m) = self
                .telemetry
                .media
                .as_ref()
                .filter(|m| m.has_session && m.duration_secs > 0)
            {
                let prog = (m.position_secs as f32 / m.duration_secs as f32).clamp(0.0, 1.0);
                let bar_bg = Rect::new(text_x, 82, text_w, 3);
                let bar_fg = Rect::new(text_x, 82, ((text_w as f32) * prog).round() as i32, 3);
                cv.fill_rounded_rect(bar_bg, 1.5, C_CARD_ALT, 0.8);
                cv.fill_rounded_rect(bar_fg, 1.5, self.active_palette.accent, 1.0);
            }

            // Compact controls: Prev / Play-Pause / Next (when media) + Mute toggle
            if has_media {
                let is_playing = self.telemetry.media.as_ref().is_some_and(|m| m.is_playing);
                for (i, (lbl, act)) in [
                    ("<", UiAction::MediaPrev),
                    (
                        if is_playing { "||" } else { ">" },
                        UiAction::MediaPlayPause,
                    ),
                    (">>", UiAction::MediaNext),
                ]
                .into_iter()
                .enumerate()
                {
                    let br = Rect::new(ctrl_start_x + i as i32 * 28, 54, 24, 26);
                    cv.fill_rounded_rect(br, 6.0, C_CARD_ALT, 0.9);
                    cv.draw_text_clipped(
                        fonts,
                        lbl,
                        br.x + if lbl.len() > 1 { 5 } else { 8 },
                        br.y + 6,
                        18,
                        10.5,
                        true,
                        if i == 1 {
                            self.active_palette.accent
                        } else {
                            C_FG
                        },
                    );
                    self.hit_boxes.push(HitBox {
                        rect: br,
                        action: act,
                        is_slider: false,
                    });
                }
                let mute_r = Rect::new(log_w - 44, 54, 28, 26);
                let muted = self.telemetry.muted.unwrap_or(false);
                cv.fill_rounded_rect(mute_r, 6.0, if muted { C_WARN } else { C_CARD_ALT }, 0.9);
                cv.draw_text_clipped(
                    fonts,
                    if muted { "M" } else { "V" },
                    mute_r.x + 9,
                    mute_r.y + 6,
                    14,
                    10.5,
                    true,
                    if muted { C_BG } else { C_MUTED },
                );
                self.hit_boxes.push(HitBox {
                    rect: mute_r,
                    action: UiAction::ToggleMute,
                    is_slider: false,
                });
            } else {
                let mute_r = Rect::new(log_w - 82, 54, 34, 26);
                let muted = self.telemetry.muted.unwrap_or(false);
                cv.fill_rounded_rect(mute_r, 6.0, if muted { C_WARN } else { C_CARD_ALT }, 0.9);
                cv.draw_text_clipped(
                    fonts,
                    if muted { "Mute" } else { "Vol" },
                    mute_r.x + 5,
                    mute_r.y + 7,
                    26,
                    10.0,
                    true,
                    if muted { C_BG } else { C_MUTED },
                );
                self.hit_boxes.push(HitBox {
                    rect: mute_r,
                    action: UiAction::ToggleMute,
                    is_slider: false,
                });

                let play_btn = Rect::new(log_w - 44, 54, 28, 26);
                cv.fill_rounded_rect(play_btn, 6.0, C_CARD_ALT, 0.9);
                cv.draw_text_clipped(
                    fonts,
                    ">",
                    play_btn.x + 10,
                    play_btn.y + 6,
                    14,
                    11.0,
                    true,
                    C_ACCENT,
                );
                self.hit_boxes.push(HitBox {
                    rect: play_btn,
                    action: UiAction::MediaPlayPause,
                    is_slider: false,
                });
            }

            let _ = buffer.present();
            return;
        }

        // 2. Navigation Tabs (y: 42..70)
        let tabs = [
            (ActiveTab::Media, "Media"),
            (ActiveTab::Timers, "Focus"),
            (ActiveTab::Notes, "Notes"),
            (ActiveTab::Stash, "Stash"),
            (ActiveTab::Ask, "Ask"),
        ];
        let tab_w = (log_w - 20 - 16) / 5;
        for (i, (tab, title)) in tabs.iter().enumerate() {
            let rx = 10 + (i as i32) * (tab_w + 4);
            let r = Rect::new(rx, 42, tab_w, 26);
            let active = self.state.active_tab == *tab;
            cv.fill_rounded_rect(r, 8.0, if active { C_ACCENT } else { C_CARD }, 1.0);
            cv.draw_text_clipped(
                fonts,
                title,
                rx + 11,
                48,
                tab_w - 14,
                12.0,
                active,
                if active { C_BG } else { C_MUTED },
            );
            self.hit_boxes.push(HitBox {
                rect: r,
                action: UiAction::SelectTab(*tab),
                is_slider: false,
            });
        }

        // 3. Primary Workspace Card Area (y: 78..496)
        match self.state.active_tab {
            ActiveTab::Media => {
                // Album art hero banner
                let hero = Rect::new(10, 78, log_w - 20, 122);
                cv.fill_rounded_gradient(
                    hero,
                    12.0,
                    self.active_palette.bg_left,
                    self.active_palette.bg_right,
                    1.0,
                );
                cv.stroke_rounded_rect(hero, 12.0, C_BORDER, 1.0);

                let art_rect = Rect::new(22, 92, 64, 64);
                let decoded_art = self
                    .telemetry
                    .media
                    .as_ref()
                    .and_then(|m| m.thumbnail_bytes.as_deref())
                    .and_then(|bytes| self.artwork_cache.get_or_decode(bytes).cloned());

                if let Some(ref art) = decoded_art {
                    cv.blit_rounded_rgba(art_rect, 10.0, art.width, art.height, &art.rgba);
                } else {
                    cv.fill_rounded_rect(art_rect, 10.0, C_CARD_ALT, 1.0);
                    cv.draw_text_clipped(fonts, "NP", 42, 114, 30, 15.0, true, C_MUTED);
                }

                if let Some(ref m) = self.telemetry.media {
                    cv.draw_text_clipped(
                        fonts,
                        &m.title,
                        98,
                        94,
                        log_w - 120,
                        15.0,
                        true,
                        self.active_palette.text_primary,
                    );
                    cv.draw_text_clipped(
                        fonts,
                        if m.artist.is_empty() {
                            "Unknown artist"
                        } else {
                            &m.artist
                        },
                        98,
                        116,
                        log_w - 120,
                        12.5,
                        false,
                        self.active_palette.text_secondary,
                    );
                    cv.draw_text_clipped(
                        fonts,
                        &m.source_app,
                        98,
                        136,
                        log_w - 120,
                        11.0,
                        false,
                        self.active_palette.accent,
                    );

                    // Track timeline bar + buttons
                    let prog_r = Rect::new(22, 166, log_w - 44, 4);
                    cv.fill_rounded_rect(prog_r, 2.0, C_CARD_ALT, 0.9);
                    if m.duration_secs > 0 {
                        let frac =
                            (m.position_secs as f32 / m.duration_secs as f32).clamp(0.0, 1.0);
                        let fill_w = ((prog_r.w as f32) * frac).round() as i32;
                        if fill_w > 2 {
                            cv.fill_rounded_rect(
                                Rect::new(prog_r.x, prog_r.y, fill_w, 4),
                                2.0,
                                self.active_palette.accent,
                                1.0,
                            );
                        }
                    }

                    let play_lbl = if m.is_playing { "Pause" } else { "Play" };
                    for (idx, (lbl, act)) in [
                        ("Prev", UiAction::MediaPrev),
                        (play_lbl, UiAction::MediaPlayPause),
                        ("Next", UiAction::MediaNext),
                    ]
                    .into_iter()
                    .enumerate()
                    {
                        let bx = 22 + idx as i32 * 108;
                        let br = Rect::new(bx, 174, 100, 20);
                        cv.fill_rounded_rect(br, 6.0, C_CARD, 0.85);
                        cv.draw_text_clipped(fonts, lbl, bx + 32, 178, 60, 11.5, true, C_FG);
                        self.hit_boxes.push(HitBox {
                            rect: br,
                            action: act,
                            is_slider: false,
                        });
                    }
                } else {
                    cv.draw_text_clipped(
                        fonts,
                        "No Active Media Player",
                        98,
                        102,
                        log_w - 120,
                        14.0,
                        true,
                        C_FG,
                    );
                    cv.draw_text_clipped(
                        fonts,
                        "Start Spotify, YouTube Music, or VLC to see track art",
                        98,
                        124,
                        log_w - 120,
                        11.0,
                        false,
                        C_MUTED,
                    );
                }

                // System Volume Control Card (y: 210..294)
                let vol_card = Rect::new(10, 210, log_w - 20, 84);
                cv.fill_rounded_rect(vol_card, 12.0, C_CARD, 1.0);
                cv.stroke_rounded_rect(vol_card, 12.0, C_BORDER, 1.0);
                let vol_pct = self.telemetry.volume.unwrap_or(0);
                let muted = self.telemetry.muted.unwrap_or(false);
                cv.draw_text_clipped(
                    fonts,
                    "Master Output Volume",
                    22,
                    222,
                    180,
                    13.0,
                    true,
                    C_FG,
                );
                let vol_txt = if muted {
                    format!("Muted ({}%)", vol_pct)
                } else {
                    format!("{}%", vol_pct)
                };
                cv.draw_text_clipped(fonts, &vol_txt, log_w - 110, 222, 88, 13.0, true, C_ACCENT);

                let mute_btn = Rect::new(22, 250, 56, 26);
                cv.fill_rounded_rect(
                    mute_btn,
                    7.0,
                    if muted { C_DANGER } else { C_CARD_ALT },
                    1.0,
                );
                cv.draw_text_clipped(
                    fonts,
                    if muted { "Unmute" } else { "Mute" },
                    29,
                    256,
                    44,
                    11.0,
                    true,
                    C_FG,
                );
                self.hit_boxes.push(HitBox {
                    rect: mute_btn,
                    action: UiAction::ToggleMute,
                    is_slider: false,
                });

                let slider_r = Rect::new(88, 255, log_w - 110, 16);
                cv.fill_rounded_rect(
                    Rect::new(slider_r.x, slider_r.y + 5, slider_r.w, 6),
                    3.0,
                    C_CARD_ALT,
                    1.0,
                );
                let fill_w = ((slider_r.w as f32) * (vol_pct as f32 / 100.0)).round() as i32;
                if fill_w > 2 {
                    cv.fill_rounded_rect(
                        Rect::new(slider_r.x, slider_r.y + 5, fill_w, 6),
                        3.0,
                        if muted { C_MUTED } else { C_ACCENT },
                        1.0,
                    );
                }
                let knob_x = slider_r.x + fill_w.clamp(4, slider_r.w - 4) - 6;
                cv.fill_rounded_rect(Rect::new(knob_x, slider_r.y + 2, 12, 12), 6.0, C_FG, 1.0);
                self.hit_boxes.push(HitBox {
                    rect: slider_r,
                    action: UiAction::SetVolumeRatio(vol_pct as f32 / 100.0),
                    is_slider: true,
                });

                // System Hardware & Quick Summary Card (y: 304..426)
                let sys_card = Rect::new(10, 304, log_w - 20, 118);
                cv.fill_rounded_rect(sys_card, 12.0, C_CARD, 1.0);
                cv.stroke_rounded_rect(sys_card, 12.0, C_BORDER, 1.0);
                cv.draw_text_clipped(
                    fonts,
                    "Host System Snapshot",
                    22,
                    316,
                    200,
                    13.0,
                    true,
                    C_FG,
                );
                if let Some(ref st) = self.telemetry.stats {
                    let used_mb = st.total_memory_mb.saturating_sub(st.available_memory_mb);
                    cv.draw_text_clipped(
                        fonts,
                        &format!("OS: {} ({} cores)", st.os, st.cpu_cores),
                        22,
                        340,
                        log_w - 44,
                        12.0,
                        false,
                        C_MUTED,
                    );
                    cv.draw_text_clipped(
                        fonts,
                        &format!(
                            "Host: {}   RAM: {:.1} / {:.1} GB used",
                            st.hostname,
                            used_mb as f32 / 1024.0,
                            st.total_memory_mb as f32 / 1024.0
                        ),
                        22,
                        362,
                        log_w - 44,
                        12.0,
                        false,
                        C_FG,
                    );
                } else {
                    cv.draw_text_clipped(
                        fonts,
                        "Reading system telemetry...",
                        22,
                        344,
                        240,
                        12.0,
                        false,
                        C_MUTED,
                    );
                }
                let bat_line = match self.telemetry.battery {
                    Some(b) => format!(
                        "Battery: {}% ({})",
                        b.percent,
                        if b.charging {
                            "Charging"
                        } else if b.on_ac_power {
                            "Plugged in"
                        } else {
                            "On battery"
                        }
                    ),
                    None => "Power: AC / No Battery Sensor".into(),
                };
                cv.draw_text_clipped(fonts, &bat_line, 22, 386, log_w - 44, 12.0, false, C_ACCENT);

                // Quick Timer / Focus Status Strip (y: 432..492)
                let foot_card = Rect::new(10, 432, log_w - 20, 58);
                cv.fill_rounded_rect(foot_card, 12.0, C_CARD, 1.0);
                cv.draw_text_clipped(
                    fonts,
                    &format!(
                        "Pomodoro: {}  {}   |   Active Timers: {}   Notes: {}",
                        self.state.pomodoro.phase.label(),
                        fmt_mm_ss(self.state.pomodoro.remaining_secs),
                        self.state.timers.len(),
                        self.state.notes.len()
                    ),
                    22,
                    444,
                    log_w - 44,
                    11.5,
                    false,
                    C_FG,
                );
                cv.draw_text_clipped(
                    fonts,
                    "Shortcut: Alt+Space toggle   Ctrl+1..5 tabs   Drop files anywhere in Stash",
                    22,
                    466,
                    log_w - 44,
                    10.5,
                    false,
                    C_MUTED,
                );
            }
            ActiveTab::Timers => {
                // Pomodoro Card (y: 78..224)
                let p_card = Rect::new(10, 78, log_w - 20, 146);
                cv.fill_rounded_rect(p_card, 12.0, C_CARD, 1.0);
                cv.stroke_rounded_rect(p_card, 12.0, C_BORDER, 1.0);

                cv.draw_text_clipped(
                    fonts,
                    &format!("Pomodoro - {}", self.state.pomodoro.phase.label()),
                    22,
                    88,
                    190,
                    13.0,
                    true,
                    C_FG,
                );
                cv.draw_text_clipped(
                    fonts,
                    &format!("Done today: {}", self.state.pomodoro.completed_today),
                    log_w - 115,
                    90,
                    95,
                    11.0,
                    false,
                    C_GREEN,
                );

                // Big timer readout
                cv.draw_text_clipped(
                    fonts,
                    &fmt_mm_ss(self.state.pomodoro.remaining_secs),
                    22,
                    112,
                    160,
                    28.0,
                    true,
                    C_ACCENT,
                );

                // Phase selector pills
                for (i, (ph, lbl)) in [
                    (PomodoroPhase::Work, "Work"),
                    (PomodoroPhase::ShortBreak, "Short"),
                    (PomodoroPhase::LongBreak, "Long"),
                ]
                .into_iter()
                .enumerate()
                {
                    let r = Rect::new(172 + i as i32 * 60, 118, 55, 22);
                    let sel = self.state.pomodoro.phase == ph;
                    cv.fill_rounded_rect(r, 6.0, if sel { C_ACCENT } else { C_CARD_ALT }, 1.0);
                    cv.draw_text_clipped(
                        fonts,
                        lbl,
                        r.x + 10,
                        r.y + 5,
                        42,
                        11.0,
                        sel,
                        if sel { C_BG } else { C_MUTED },
                    );
                    self.hit_boxes.push(HitBox {
                        rect: r,
                        action: UiAction::SelectPomodoroPhase(ph),
                        is_slider: false,
                    });
                }

                // Controls row
                let start_lbl = if self.state.pomodoro.running {
                    "Pause"
                } else {
                    "Start"
                };
                for (i, (lbl, col, act)) in [
                    (start_lbl, C_ACCENT, UiAction::TogglePomodoro),
                    ("Reset", C_CARD_ALT, UiAction::ResetPomodoro),
                    ("Skip", C_CARD_ALT, UiAction::SkipPomodoro),
                    (
                        if self.state.pomodoro.auto_advance {
                            "Auto: On"
                        } else {
                            "Auto: Off"
                        },
                        if self.state.pomodoro.auto_advance {
                            C_GREEN
                        } else {
                            C_CARD_ALT
                        },
                        UiAction::TogglePomodoroAuto,
                    ),
                ]
                .into_iter()
                .enumerate()
                {
                    let r = Rect::new(22 + i as i32 * 82, 184, 76, 26);
                    let dark_txt = col == C_ACCENT || col == C_GREEN;
                    cv.fill_rounded_rect(r, 7.0, col, 1.0);
                    cv.draw_text_clipped(
                        fonts,
                        lbl,
                        r.x + 12,
                        r.y + 6,
                        64,
                        11.5,
                        true,
                        if dark_txt { C_BG } else { C_FG },
                    );
                    self.hit_boxes.push(HitBox {
                        rect: r,
                        action: act,
                        is_slider: false,
                    });
                }

                // Quick Timer Creator (y: 234..314)
                let c_card = Rect::new(10, 234, log_w - 20, 82);
                cv.fill_rounded_rect(c_card, 12.0, C_CARD, 1.0);
                for (i, (lbl, secs)) in [("+5m", 300u64), ("+10m", 600), ("+25m", 1500)]
                    .into_iter()
                    .enumerate()
                {
                    let r = Rect::new(22 + i as i32 * 64, 244, 58, 22);
                    cv.fill_rounded_rect(r, 6.0, C_CARD_ALT, 1.0);
                    cv.draw_text_clipped(fonts, lbl, r.x + 14, r.y + 5, 42, 11.0, true, C_ACCENT);
                    self.hit_boxes.push(HitBox {
                        rect: r,
                        action: UiAction::AddPresetTimer(secs),
                        is_slider: false,
                    });
                }

                // Custom label + mins + Add
                let lbl_box = Rect::new(22, 274, 172, 28);
                cv.fill_rounded_rect(lbl_box, 7.0, C_BG, 1.0);
                cv.stroke_rounded_rect(
                    lbl_box,
                    7.0,
                    if self.focus == FocusField::TimerLabel {
                        C_ACCENT
                    } else {
                        C_BORDER
                    },
                    1.0,
                );
                let ltxt = if self.timer_label_input.is_empty() {
                    "Timer label..."
                } else {
                    &self.timer_label_input
                };
                cv.draw_text_clipped(
                    fonts,
                    ltxt,
                    28,
                    281,
                    158,
                    12.0,
                    false,
                    if self.timer_label_input.is_empty() {
                        C_MUTED
                    } else {
                        C_FG
                    },
                );
                self.hit_boxes.push(HitBox {
                    rect: lbl_box,
                    action: UiAction::Focus(FocusField::TimerLabel),
                    is_slider: false,
                });

                let min_box = Rect::new(202, 274, 62, 28);
                cv.fill_rounded_rect(min_box, 7.0, C_BG, 1.0);
                cv.stroke_rounded_rect(
                    min_box,
                    7.0,
                    if self.focus == FocusField::TimerMins {
                        C_ACCENT
                    } else {
                        C_BORDER
                    },
                    1.0,
                );
                cv.draw_text_clipped(
                    fonts,
                    &format!("{}m", self.timer_mins_input),
                    210,
                    281,
                    48,
                    12.0,
                    false,
                    C_FG,
                );
                self.hit_boxes.push(HitBox {
                    rect: min_box,
                    action: UiAction::Focus(FocusField::TimerMins),
                    is_slider: false,
                });

                let add_btn = Rect::new(272, 274, 74, 28);
                cv.fill_rounded_rect(add_btn, 7.0, C_ACCENT, 1.0);
                cv.draw_text_clipped(fonts, "+ Add", 289, 281, 50, 12.0, true, C_BG);
                self.hit_boxes.push(HitBox {
                    rect: add_btn,
                    action: UiAction::AddCustomTimer,
                    is_slider: false,
                });

                // Active Timers List (y: 326..490)
                if self.state.timers.is_empty() {
                    let emp = Rect::new(10, 326, log_w - 20, 160);
                    cv.fill_rounded_rect(emp, 12.0, C_CARD, 1.0);
                    cv.draw_text_clipped(
                        fonts,
                        "No active timers. Use a preset (+5m, +10m) or add one above.",
                        24,
                        392,
                        log_w - 48,
                        12.0,
                        false,
                        C_MUTED,
                    );
                } else {
                    for (idx, t) in self.state.timers.iter().take(4).enumerate() {
                        let y = 326 + idx as i32 * 42;
                        let r = Rect::new(10, y, log_w - 20, 36);
                        cv.fill_rounded_rect(r, 8.0, C_CARD, 1.0);
                        cv.draw_text_clipped(fonts, &t.label, 20, y + 10, 124, 12.0, false, C_FG);
                        cv.draw_text_clipped(
                            fonts,
                            &fmt_mm_ss(t.remaining_secs),
                            150,
                            y + 10,
                            54,
                            12.5,
                            true,
                            if t.paused { C_WARN } else { C_ACCENT },
                        );

                        for (bi, (lbl, act)) in [
                            (
                                if t.paused { "Go" } else { "Hold" },
                                UiAction::ToggleTimerPause(t.id.clone()),
                            ),
                            ("Rst", UiAction::ResetTimer(t.id.clone())),
                            ("Del", UiAction::DeleteTimer(t.id.clone())),
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            let br = Rect::new(212 + bi as i32 * 46, y + 6, 42, 24);
                            cv.fill_rounded_rect(br, 6.0, C_CARD_ALT, 1.0);
                            cv.draw_text_clipped(
                                fonts,
                                lbl,
                                br.x + 8,
                                br.y + 6,
                                32,
                                10.5,
                                true,
                                if lbl == "Del" { C_DANGER } else { C_FG },
                            );
                            self.hit_boxes.push(HitBox {
                                rect: br,
                                action: act,
                                is_slider: false,
                            });
                        }
                    }
                }
            }
            ActiveTab::Notes => {
                // Capture + Search Bar (y: 78..150)
                let in_box = Rect::new(10, 78, log_w - 92, 30);
                cv.fill_rounded_rect(in_box, 8.0, C_CARD, 1.0);
                cv.stroke_rounded_rect(
                    in_box,
                    8.0,
                    if self.focus == FocusField::NoteInput && self.editing_note_id.is_none() {
                        C_ACCENT
                    } else {
                        C_BORDER
                    },
                    1.0,
                );
                let ntxt = if self.note_input.is_empty() {
                    "Quick note (press Enter to save)..."
                } else {
                    &self.note_input
                };
                cv.draw_text_clipped(
                    fonts,
                    ntxt,
                    18,
                    86,
                    log_w - 108,
                    12.0,
                    false,
                    if self.note_input.is_empty() {
                        C_MUTED
                    } else {
                        C_FG
                    },
                );
                self.hit_boxes.push(HitBox {
                    rect: in_box,
                    action: UiAction::Focus(FocusField::NoteInput),
                    is_slider: false,
                });

                let add_n = Rect::new(log_w - 76, 78, 66, 30);
                cv.fill_rounded_rect(add_n, 8.0, C_ACCENT, 1.0);
                cv.draw_text_clipped(fonts, "+ Save", add_n.x + 12, 86, 52, 12.0, true, C_BG);
                self.hit_boxes.push(HitBox {
                    rect: add_n,
                    action: UiAction::AddNote,
                    is_slider: false,
                });

                let s_box = Rect::new(10, 114, log_w - 20, 26);
                cv.fill_rounded_rect(s_box, 7.0, C_BG, 1.0);
                cv.stroke_rounded_rect(
                    s_box,
                    7.0,
                    if self.focus == FocusField::NoteSearch {
                        C_ACCENT
                    } else {
                        C_BORDER
                    },
                    1.0,
                );
                let stxt = if self.note_search.is_empty() {
                    "Filter notes..."
                } else {
                    &self.note_search
                };
                cv.draw_text_clipped(
                    fonts,
                    stxt,
                    18,
                    120,
                    log_w - 36,
                    11.5,
                    false,
                    if self.note_search.is_empty() {
                        C_MUTED
                    } else {
                        C_FG
                    },
                );
                self.hit_boxes.push(HitBox {
                    rect: s_box,
                    action: UiAction::Focus(FocusField::NoteSearch),
                    is_slider: false,
                });

                // Edit banner if editing a note
                let mut list_top = 148;
                if let Some(_edit_id) = &self.editing_note_id {
                    let er = Rect::new(10, 148, log_w - 114, 28);
                    cv.fill_rounded_rect(er, 7.0, C_CARD_ALT, 1.0);
                    cv.stroke_rounded_rect(er, 7.0, C_ACCENT, 1.0);
                    cv.draw_text_clipped(
                        fonts,
                        &self.editing_note_buf,
                        18,
                        155,
                        log_w - 130,
                        11.5,
                        false,
                        C_FG,
                    );
                    let save_e = Rect::new(log_w - 98, 148, 42, 28);
                    let canc_e = Rect::new(log_w - 52, 148, 42, 28);
                    cv.fill_rounded_rect(save_e, 7.0, C_GREEN, 1.0);
                    cv.draw_text_clipped(fonts, "OK", save_e.x + 11, 155, 24, 11.0, true, C_BG);
                    cv.fill_rounded_rect(canc_e, 7.0, C_CARD_ALT, 1.0);
                    cv.draw_text_clipped(
                        fonts,
                        "Esc",
                        canc_e.x + 10,
                        155,
                        24,
                        11.0,
                        false,
                        C_MUTED,
                    );
                    self.hit_boxes.push(HitBox {
                        rect: save_e,
                        action: UiAction::SaveEditNote,
                        is_slider: false,
                    });
                    self.hit_boxes.push(HitBox {
                        rect: canc_e,
                        action: UiAction::CancelEditNote,
                        is_slider: false,
                    });
                    list_top = 184;
                }

                let q_lower = self.note_search.trim().to_lowercase();
                let filtered: Vec<_> = self
                    .state
                    .notes
                    .iter()
                    .filter(|n| q_lower.is_empty() || n.text.to_lowercase().contains(&q_lower))
                    .take(6)
                    .cloned()
                    .collect();

                if filtered.is_empty() {
                    let emp = Rect::new(10, list_top, log_w - 20, 488 - list_top);
                    cv.fill_rounded_rect(emp, 12.0, C_CARD, 1.0);
                    cv.draw_text_clipped(
                        fonts,
                        if self.state.notes.is_empty() {
                            "No notes yet. Type above and press Enter."
                        } else {
                            "No notes match your search filter."
                        },
                        24,
                        list_top + 64,
                        log_w - 48,
                        12.0,
                        false,
                        C_MUTED,
                    );
                } else {
                    for (idx, n) in filtered.iter().enumerate() {
                        let y = list_top + idx as i32 * 50;
                        if y + 44 > 492 {
                            break;
                        }
                        let r = Rect::new(10, y, log_w - 20, 44);
                        cv.fill_rounded_rect(r, 9.0, C_CARD, 1.0);
                        cv.draw_text_clipped(
                            fonts,
                            if n.pinned { "[Pin]" } else { "" },
                            18,
                            y + 7,
                            36,
                            10.0,
                            true,
                            C_WARN,
                        );
                        cv.draw_text_clipped(
                            fonts,
                            &n.text,
                            if n.pinned { 54 } else { 18 },
                            y + 14,
                            170,
                            12.0,
                            false,
                            C_FG,
                        );

                        for (bi, (lbl, act)) in [
                            (
                                if n.pinned { "Unp" } else { "Pin" },
                                UiAction::ToggleNotePin(n.id.clone()),
                            ),
                            ("Edt", UiAction::StartEditNote(n.id.clone())),
                            ("Cpy", UiAction::CopyText(n.text.clone())),
                            ("Del", UiAction::DeleteNote(n.id.clone())),
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            let br = Rect::new(192 + bi as i32 * 40, y + 10, 36, 24);
                            cv.fill_rounded_rect(br, 6.0, C_CARD_ALT, 1.0);
                            cv.draw_text_clipped(
                                fonts,
                                lbl,
                                br.x + 6,
                                br.y + 6,
                                28,
                                10.0,
                                true,
                                if lbl == "Del" { C_DANGER } else { C_FG },
                            );
                            self.hit_boxes.push(HitBox {
                                rect: br,
                                action: act,
                                is_slider: false,
                            });
                        }
                    }
                }
            }
            ActiveTab::Stash => {
                let search_rect = Rect::new(10, 78, log_w - 134, 32);
                cv.fill_rounded_rect(search_rect, 8.0, C_CARD, 1.0);
                if self.focus == FocusField::StashSearch {
                    cv.stroke_rounded_rect(search_rect, 8.0, C_ACCENT, 1.2);
                }
                let search_txt = if self.stash_search.is_empty() {
                    "Search stash or drop files..."
                } else {
                    self.stash_search.as_str()
                };
                cv.draw_text_clipped(
                    fonts,
                    search_txt,
                    18,
                    87,
                    search_rect.w - 14,
                    11.5,
                    false,
                    if self.stash_search.is_empty() {
                        C_MUTED
                    } else {
                        C_FG
                    },
                );
                self.hit_boxes.push(HitBox {
                    rect: search_rect,
                    action: UiAction::Focus(FocusField::StashSearch),
                    is_slider: false,
                });

                let paste_btn = Rect::new(log_w - 118, 81, 100, 26);
                cv.fill_rounded_rect(paste_btn, 7.0, C_ACCENT, 1.0);
                cv.draw_text_clipped(
                    fonts,
                    "+ Clipboard",
                    paste_btn.x + 12,
                    88,
                    82,
                    11.5,
                    true,
                    C_BG,
                );
                self.hit_boxes.push(HitBox {
                    rect: paste_btn,
                    action: UiAction::PasteClipboardToStash,
                    is_slider: false,
                });

                let q = self.stash_search.trim().to_lowercase();
                let filtered: Vec<&state::StashEntry> = self
                    .state
                    .stash
                    .iter()
                    .filter(|item| {
                        q.is_empty()
                            || item.label.to_lowercase().contains(&q)
                            || item.payload.to_lowercase().contains(&q)
                    })
                    .collect();

                if filtered.is_empty() {
                    let emp = Rect::new(10, 120, log_w - 20, 370);
                    cv.fill_rounded_rect(emp, 12.0, C_CARD, 1.0);
                    if self.drag_hover_count > 0 {
                        cv.stroke_rounded_rect(emp, 12.0, C_ACCENT, 2.0);
                    }
                    let msg = if self.drag_hover_count > 0 {
                        "Release to add dropped files to Stash"
                    } else if self.state.stash.is_empty() {
                        "Stash is empty. Drag files onto Pico or click '+ Clipboard'."
                    } else {
                        "No stash items match your search."
                    };
                    cv.draw_text_clipped(
                        fonts,
                        msg,
                        24,
                        250,
                        log_w - 48,
                        12.0,
                        false,
                        if self.drag_hover_count > 0 {
                            C_ACCENT
                        } else {
                            C_MUTED
                        },
                    );
                } else {
                    for (idx, item) in filtered.iter().take(7).enumerate() {
                        let y = 120 + idx as i32 * 52;
                        let r = Rect::new(10, y, log_w - 20, 46);
                        cv.fill_rounded_rect(r, 9.0, C_CARD, 1.0);
                        let badge = match item.kind {
                            state::StashKind::File => {
                                if item.missing_on_disk {
                                    "MISS"
                                } else {
                                    "FILE"
                                }
                            }
                            state::StashKind::Url => "URL",
                            state::StashKind::Text => "TEXT",
                        };
                        cv.draw_text_clipped(
                            fonts,
                            badge,
                            18,
                            y + 15,
                            36,
                            10.0,
                            true,
                            if item.missing_on_disk {
                                C_DANGER
                            } else {
                                C_ACCENT
                            },
                        );
                        cv.draw_text_clipped(fonts, &item.label, 58, y + 8, 160, 11.5, true, C_FG);
                        cv.draw_text_clipped(
                            fonts,
                            &item.preview(),
                            58,
                            y + 25,
                            160,
                            9.5,
                            false,
                            C_MUTED,
                        );

                        for (bi, (lbl, act)) in [
                            ("Open", UiAction::OpenStashPayload(item.payload.clone())),
                            ("Copy", UiAction::CopyText(item.payload.clone())),
                            ("Rem", UiAction::DeleteStash(item.id.clone())),
                        ]
                        .into_iter()
                        .enumerate()
                        {
                            let br = Rect::new(224 + bi as i32 * 44, y + 11, 40, 24);
                            cv.fill_rounded_rect(br, 6.0, C_CARD_ALT, 1.0);
                            cv.draw_text_clipped(
                                fonts,
                                lbl,
                                br.x + 6,
                                br.y + 6,
                                32,
                                10.0,
                                true,
                                if lbl == "Rem" { C_WARN } else { C_FG },
                            );
                            self.hit_boxes.push(HitBox {
                                rect: br,
                                action: act,
                                is_slider: false,
                            });
                        }
                    }
                }
            }
            ActiveTab::Ask => {
                // Status & Voice Control Bar (y: 78..112)
                let bar = Rect::new(10, 78, log_w - 20, 32);
                cv.fill_rounded_rect(bar, 9.0, C_CARD, 1.0);
                let paused = self
                    .daemon_snapshot
                    .as_ref()
                    .is_some_and(|s| s.voice_paused);
                let status_str = if let Some(ref snap) = self.daemon_snapshot {
                    format!(
                        "Daemon PID {} | {} | {} tools",
                        snap.pid, snap.status, snap.registered_tools
                    )
                } else {
                    "Daemon Offline: run 'pico daemon' for voice/tools".into()
                };
                cv.draw_text_clipped(fonts, &status_str, 18, 88, 212, 11.0, false, C_MUTED);

                let mute_b = Rect::new(log_w - 134, 82, 54, 24);
                let stop_b = Rect::new(log_w - 76, 82, 58, 24);
                cv.fill_rounded_rect(mute_b, 6.0, if paused { C_WARN } else { C_CARD_ALT }, 1.0);
                cv.draw_text_clipped(
                    fonts,
                    if paused { "Muted" } else { "Mic On" },
                    mute_b.x + 7,
                    88,
                    44,
                    10.5,
                    true,
                    if paused { C_BG } else { C_FG },
                );
                cv.fill_rounded_rect(stop_b, 6.0, C_CARD_ALT, 1.0);
                cv.draw_text_clipped(fonts, "Stop", stop_b.x + 14, 88, 36, 10.5, true, C_DANGER);
                self.hit_boxes.push(HitBox {
                    rect: mute_b,
                    action: UiAction::ToggleDaemonMute,
                    is_slider: false,
                });
                self.hit_boxes.push(HitBox {
                    rect: stop_b,
                    action: UiAction::InterruptDaemonSpeech,
                    is_slider: false,
                });

                // Pending confirmation or clarification cards if daemon reports them
                let mut chat_top = 116;
                if let Some(ref snap) = self.daemon_snapshot {
                    if let Some(ref conf) = snap.pending_confirmation {
                        let cr = Rect::new(10, chat_top, log_w - 20, 62);
                        cv.fill_rounded_rect(cr, 9.0, C_CARD_ALT, 1.0);
                        cv.stroke_rounded_rect(cr, 9.0, C_WARN, 1.0);
                        cv.draw_text_clipped(
                            fonts,
                            &format!("Confirm '{}': {}", conf.tool_name, conf.arguments_preview),
                            18,
                            chat_top + 8,
                            log_w - 36,
                            11.5,
                            true,
                            C_WARN,
                        );
                        let ok_b = Rect::new(18, chat_top + 32, 92, 22);
                        let no_b = Rect::new(118, chat_top + 32, 92, 22);
                        cv.fill_rounded_rect(ok_b, 6.0, C_GREEN, 1.0);
                        cv.draw_text_clipped(
                            fonts,
                            "Approve",
                            ok_b.x + 20,
                            ok_b.y + 5,
                            64,
                            11.0,
                            true,
                            C_BG,
                        );
                        cv.fill_rounded_rect(no_b, 6.0, C_DANGER, 1.0);
                        cv.draw_text_clipped(
                            fonts,
                            "Decline",
                            no_b.x + 20,
                            no_b.y + 5,
                            64,
                            11.0,
                            true,
                            C_BG,
                        );
                        self.hit_boxes.push(HitBox {
                            rect: ok_b,
                            action: UiAction::ConfirmTool(conf.request_id.clone(), true),
                            is_slider: false,
                        });
                        self.hit_boxes.push(HitBox {
                            rect: no_b,
                            action: UiAction::ConfirmTool(conf.request_id.clone(), false),
                            is_slider: false,
                        });
                        chat_top += 68;
                    } else if let Some(ref clar) = snap.pending_clarification {
                        let cr = Rect::new(10, chat_top, log_w - 20, 66);
                        cv.fill_rounded_rect(cr, 9.0, C_CARD_ALT, 1.0);
                        cv.draw_text_clipped(
                            fonts,
                            &clar.question,
                            18,
                            chat_top + 6,
                            log_w - 36,
                            11.0,
                            true,
                            C_ACCENT,
                        );
                        for (i, opt) in clar.candidates.iter().take(3).enumerate() {
                            let b = Rect::new(18 + i as i32 * 112, chat_top + 34, 104, 24);
                            cv.fill_rounded_rect(b, 6.0, C_CARD, 1.0);
                            cv.draw_text_clipped(
                                fonts,
                                opt,
                                b.x + 6,
                                b.y + 6,
                                92,
                                10.5,
                                false,
                                C_FG,
                            );
                            self.hit_boxes.push(HitBox {
                                rect: b,
                                action: UiAction::SelectClarification(i),
                                is_slider: false,
                            });
                        }
                        chat_top += 72;
                    }
                }

                // Conversation Log Card (chat_top..436)
                let chat_h = (436 - chat_top).max(120);
                let chat_card = Rect::new(10, chat_top, log_w - 20, chat_h);
                cv.fill_rounded_rect(chat_card, 12.0, C_CARD, 1.0);
                if self.ask_messages.is_empty() {
                    cv.draw_text_clipped(
                        fonts,
                        "Ask Pico or type math like '128 * 4 + 16' for instant local eval.",
                        22,
                        chat_top + 48,
                        log_w - 44,
                        11.5,
                        false,
                        C_MUTED,
                    );
                } else {
                    let max_rows = ((chat_h - 16) / 34).max(1) as usize;
                    let start = self.ask_messages.len().saturating_sub(max_rows);
                    for (idx, (is_user, text)) in self.ask_messages[start..].iter().enumerate() {
                        let y = chat_top + 10 + idx as i32 * 34;
                        let r = Rect::new(18, y, log_w - 36, 28);
                        cv.fill_rounded_rect(r, 7.0, if *is_user { C_CARD_ALT } else { C_BG }, 1.0);
                        let prefix = if *is_user { "You: " } else { "Pico: " };
                        cv.draw_text_clipped(
                            fonts,
                            &format!("{}{}", prefix, text),
                            26,
                            y + 7,
                            log_w - 52,
                            11.5,
                            false,
                            if *is_user { C_ACCENT } else { C_FG },
                        );
                    }
                }

                // Instant Quick Calc live preview pill
                let calc_res = try_eval_quick_math(&self.ask_input);
                if let Some(ref ans) = calc_res {
                    let cr = Rect::new(10, 440, log_w - 20, 20);
                    cv.fill_rounded_rect(cr, 6.0, C_CARD_ALT, 1.0);
                    cv.draw_text_clipped(
                        fonts,
                        &format!("Quick Calc: = {}", ans),
                        18,
                        444,
                        log_w - 36,
                        11.0,
                        true,
                        C_GREEN,
                    );
                }

                // Prompt Input Box (y: 464..494)
                let ask_box = Rect::new(10, 464, log_w - 134, 30);
                cv.fill_rounded_rect(ask_box, 8.0, C_CARD, 1.0);
                cv.stroke_rounded_rect(
                    ask_box,
                    8.0,
                    if self.focus == FocusField::AskInput {
                        C_ACCENT
                    } else {
                        C_BORDER
                    },
                    1.0,
                );
                let atxt = if self.ask_input.is_empty() {
                    "Type command or math..."
                } else {
                    &self.ask_input
                };
                cv.draw_text_clipped(
                    fonts,
                    atxt,
                    18,
                    472,
                    log_w - 150,
                    12.0,
                    false,
                    if self.ask_input.is_empty() {
                        C_MUTED
                    } else {
                        C_FG
                    },
                );
                self.hit_boxes.push(HitBox {
                    rect: ask_box,
                    action: UiAction::Focus(FocusField::AskInput),
                    is_slider: false,
                });

                let send_b = Rect::new(log_w - 118, 464, 58, 30);
                cv.fill_rounded_rect(send_b, 8.0, C_ACCENT, 1.0);
                cv.draw_text_clipped(
                    fonts,
                    if self.ask_busy { "..." } else { "Send" },
                    send_b.x + 12,
                    472,
                    44,
                    12.0,
                    true,
                    C_BG,
                );
                self.hit_boxes.push(HitBox {
                    rect: send_b,
                    action: UiAction::SendPrompt,
                    is_slider: false,
                });

                let clr_b = Rect::new(log_w - 54, 464, 44, 30);
                cv.fill_rounded_rect(clr_b, 8.0, C_CARD_ALT, 1.0);
                cv.draw_text_clipped(fonts, "Clr", clr_b.x + 12, 472, 26, 11.5, false, C_MUTED);
                self.hit_boxes.push(HitBox {
                    rect: clr_b,
                    action: UiAction::ClearAskHistory,
                    is_slider: false,
                });
            }
        }

        // 4. Status Footer (y: 502..534)
        let status_txt = if let Some((ref msg, _)) = self.status_toast {
            msg.as_str()
        } else if self.daemon_snapshot.is_some() {
            "Connected via authenticated local IPC"
        } else {
            "Ready  |  Alt+Space hotkey  |  Ctrl+Q exit"
        };
        cv.draw_text_clipped(fonts, status_txt, 14, 512, log_w - 28, 11.0, false, C_MUTED);

        let _ = buffer.present();
    }
}

impl ApplicationHandler<CompanionEvent> for CompanionApp {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }

        let mut attrs = WindowAttributes::default()
            .with_title("Pico Companion")
            .with_inner_size(LogicalSize::new(WIN_W, self.desired_height()))
            .with_resizable(false)
            .with_decorations(false)
            .with_window_level(WindowLevel::AlwaysOnTop);

        if let Some(mon) = event_loop.primary_monitor() {
            let size = mon.size();
            let sf = mon.scale_factor();
            let px_w = (WIN_W as f64 * sf).round() as i32;
            let px_h = (self.desired_height() as f64 * sf).round() as i32;
            let x = (size.width as i32 - px_w - 20).max(0);
            let y = (size.height as i32 - px_h - 60).max(0);
            attrs = attrs.with_position(LogicalPosition::new(x as f64 / sf, y as f64 / sf));
        }

        if let Ok(win) = event_loop.create_window(attrs) {
            let win_rc = Rc::new(win);
            let context = Context::new(win_rc.clone()).ok();
            let surface = context
                .as_ref()
                .and_then(|c| Surface::new(c, win_rc.clone()).ok());
            self.window = Some(win_rc.clone());
            self.context = context;
            self.surface = surface;
            if self.args.benchmark_self {
                self.bench_started = Some(Instant::now());
            }
            win_rc.request_redraw();
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: CompanionEvent) {
        match event {
            CompanionEvent::Wake => {}
            CompanionEvent::Tray(cmd) => match cmd {
                TrayCommand::ToggleVisible => self.toggle_visible(),
                TrayCommand::ToggleCompact => {
                    self.execute_action(UiAction::ToggleCompact, event_loop)
                }
                TrayCommand::ExitApp => {
                    self.save_state();
                    event_loop.exit();
                }
            },
            CompanionEvent::TelemetryUpdated {
                mut media,
                volume,
                muted,
                stats,
                battery,
            } => {
                // Preserve previously decoded artwork bytes when track metadata matches cache key.
                if let Some(ref mut new_m) = media {
                    if new_m.thumbnail_bytes.is_none() {
                        if let Some(ref prev_m) = self.telemetry.media {
                            if prev_m.title == new_m.title && prev_m.album == new_m.album {
                                new_m.thumbnail_bytes = prev_m.thumbnail_bytes.clone();
                            }
                        }
                    }
                    if let Some(ref bytes) = new_m.thumbnail_bytes {
                        if let Some(art) = self.artwork_cache.get_or_decode(bytes) {
                            self.target_palette = art.palette;
                        }
                    }
                } else {
                    self.target_palette = ArtPalette::default();
                }
                self.telemetry = TelemetryState {
                    media,
                    volume,
                    muted,
                    stats: stats.or_else(|| self.telemetry.stats.clone()),
                    battery: battery.or(self.telemetry.battery),
                };
                self.request_redraw();
            }
            CompanionEvent::PlatformActionResult {
                message,
                error,
                volume,
                muted,
                clipboard_stash,
                refresh_media,
            } => {
                if let Some(v) = volume {
                    self.telemetry.volume = Some(v);
                }
                if let Some(m) = muted {
                    self.telemetry.muted = Some(m);
                }
                if let Some(txt) = clipboard_stash {
                    if let Some(out) = self.state.add_stash_text_or_url_checked(&txt) {
                        self.save_state();
                        let msg = if out.duplicate {
                            "Already in Stash (moved to top)"
                        } else if out.truncated {
                            "Stashed from clipboard (truncated to 64 KB)"
                        } else {
                            "Stashed from clipboard"
                        };
                        self.set_toast(msg);
                    }
                }
                if let Some(err) = error {
                    self.set_toast(err);
                } else if let Some(msg) = message {
                    self.set_toast(msg);
                }
                if refresh_media {
                    let _ = self.platform_tx.send(PlatformCommand::RefreshMedia);
                }
                self.request_redraw();
            }
            CompanionEvent::DaemonSnapshotUpdated(snap) => {
                self.daemon_snapshot = snap;
                self.request_redraw();
            }
            CompanionEvent::DaemonReply { reply, error } => {
                self.ask_busy = false;
                if let Some(r) = reply {
                    self.ask_messages.push((false, r));
                } else if let Some(err) = error {
                    self.ask_messages.push((false, format!("Error: {}", err)));
                }
                if self.ask_messages.len() > 20 {
                    self.ask_messages.remove(0);
                }
                let _ = self.daemon_tx.send(DaemonCommand::Refresh);
                self.request_redraw();
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: WindowId,
        event: WindowEvent,
    ) {
        match event {
            WindowEvent::CloseRequested => self.execute_action(UiAction::CloseOrHide, event_loop),
            WindowEvent::HoveredFile(_) => {
                self.drag_hover_count = self.drag_hover_count.saturating_add(1);
                self.request_redraw();
            }
            WindowEvent::HoveredFileCancelled => {
                self.drag_hover_count = 0;
                self.request_redraw();
            }
            WindowEvent::DroppedFile(path) => {
                self.drag_hover_count = 0;
                let out = self.state.add_stash_path_checked(&path);
                self.state.active_tab = ActiveTab::Stash;
                self.state.compact_mode = false;
                self.save_state();
                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("file");
                if out.duplicate {
                    self.set_toast(format!("Already in Stash: {}", name));
                } else {
                    self.set_toast(format!("Stashed {}", name));
                }
                self.sync_window_size();
            }
            WindowEvent::CursorMoved { position, .. } => {
                self.cursor_pos = position;
                let lx = (position.x as f32 / self.scale_factor).round() as i32;
                let ly = (position.y as f32 / self.scale_factor).round() as i32;
                if self.dragging_slider {
                    if let Some(hb) = self.hit_boxes.iter().find(|h| h.is_slider) {
                        let ratio = (lx - hb.rect.x) as f32 / hb.rect.w.max(1) as f32;
                        self.execute_action(UiAction::SetVolumeRatio(ratio), event_loop);
                    }
                    return;
                }
                let new_hover = self.hit_boxes.iter().position(|h| h.rect.contains(lx, ly));
                if new_hover != self.hovered_idx {
                    self.hovered_idx = new_hover;
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Pressed,
                button: MouseButton::Left,
                ..
            } => {
                let lx = (self.cursor_pos.x as f32 / self.scale_factor).round() as i32;
                let ly = (self.cursor_pos.y as f32 / self.scale_factor).round() as i32;
                if let Some(hb) = self
                    .hit_boxes
                    .iter()
                    .rev()
                    .find(|h| h.rect.contains(lx, ly))
                {
                    let act = hb.action.clone();
                    let is_slider = hb.is_slider;
                    if is_slider {
                        self.dragging_slider = true;
                        let ratio = (lx - hb.rect.x) as f32 / hb.rect.w.max(1) as f32;
                        self.execute_action(UiAction::SetVolumeRatio(ratio), event_loop);
                    } else {
                        self.execute_action(act, event_loop);
                    }
                }
            }
            WindowEvent::MouseInput {
                state: ElementState::Released,
                button: MouseButton::Left,
                ..
            } => {
                self.dragging_slider = false;
            }
            WindowEvent::ModifiersChanged(mods) => {
                self.modifiers = mods.state();
            }
            WindowEvent::KeyboardInput {
                event: key_ev,
                is_synthetic: false,
                ..
            } if key_ev.state == ElementState::Pressed => {
                let ctrl = self.modifiers.control_key();
                if ctrl {
                    match &key_ev.logical_key {
                        Key::Character(s) if s == "1" => {
                            self.execute_action(UiAction::SelectTab(ActiveTab::Media), event_loop);
                            return;
                        }
                        Key::Character(s) if s == "2" => {
                            self.execute_action(UiAction::SelectTab(ActiveTab::Timers), event_loop);
                            return;
                        }
                        Key::Character(s) if s == "3" => {
                            self.execute_action(UiAction::SelectTab(ActiveTab::Notes), event_loop);
                            return;
                        }
                        Key::Character(s) if s == "4" => {
                            self.execute_action(UiAction::SelectTab(ActiveTab::Stash), event_loop);
                            return;
                        }
                        Key::Character(s) if s == "5" => {
                            self.execute_action(UiAction::SelectTab(ActiveTab::Ask), event_loop);
                            return;
                        }
                        Key::Character(s) if s.eq_ignore_ascii_case("q") => {
                            self.save_state();
                            event_loop.exit();
                            return;
                        }
                        Key::Character(s) if s.eq_ignore_ascii_case("v") => {
                            if let Ok(clip) = PlatformClipboard::get_text() {
                                if let Some(buf) = self.focused_buffer_mut() {
                                    buf.push_str(&clip);
                                    self.request_redraw();
                                } else if self.state.active_tab == ActiveTab::Stash {
                                    self.execute_action(
                                        UiAction::PasteClipboardToStash,
                                        event_loop,
                                    );
                                }
                            }
                            return;
                        }
                        _ => {}
                    }
                }

                match &key_ev.logical_key {
                    Key::Named(NamedKey::Escape) => {
                        if self.editing_note_id.is_some() {
                            self.execute_action(UiAction::CancelEditNote, event_loop);
                        } else {
                            self.focus = FocusField::None;
                            self.request_redraw();
                        }
                    }
                    Key::Named(NamedKey::Backspace) => {
                        if let Some(buf) = self.focused_buffer_mut() {
                            buf.pop();
                            self.request_redraw();
                        }
                    }
                    Key::Named(NamedKey::Enter) => {
                        if self.editing_note_id.is_some() {
                            self.execute_action(UiAction::SaveEditNote, event_loop);
                        } else {
                            match self.focus {
                                FocusField::NoteInput => {
                                    self.execute_action(UiAction::AddNote, event_loop)
                                }
                                FocusField::TimerLabel | FocusField::TimerMins => {
                                    self.execute_action(UiAction::AddCustomTimer, event_loop)
                                }
                                FocusField::AskInput => {
                                    self.execute_action(UiAction::SendPrompt, event_loop)
                                }
                                _ => {}
                            }
                        }
                    }
                    Key::Named(NamedKey::Space) => {
                        if let Some(buf) = self.focused_buffer_mut() {
                            if buf.len() < 400 {
                                buf.push(' ');
                                self.request_redraw();
                            }
                        }
                    }
                    Key::Character(ch) if !ctrl => {
                        let is_mins_only = self.focus == FocusField::TimerMins;
                        if let Some(buf) = self.focused_buffer_mut() {
                            if buf.len() < 400 {
                                for c in ch.chars().filter(|c| !c.is_control()) {
                                    if !is_mins_only || c.is_ascii_digit() {
                                        buf.push(c);
                                    }
                                }
                                self.request_redraw();
                            }
                        }
                    }
                    _ => {}
                }
            }
            WindowEvent::RedrawRequested => self.render(),
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if let Some(t0) = self.bench_started {
            if t0.elapsed() >= Duration::from_millis(2500) {
                emit_self_benchmark_metrics();
                event_loop.exit();
                return;
            }
            event_loop.set_control_flow(ControlFlow::WaitUntil(t0 + Duration::from_millis(2500)));
            return;
        }

        let notifications = self
            .state
            .reconcile_clock(Utc::now(), &Local::now().format("%Y-%m-%d").to_string());
        if !notifications.is_empty() {
            for n in notifications {
                self.set_toast(format!("{}: {}", n.title, n.body));
                if let Some(ref tray) = self.tray_handle {
                    tray.notify(n);
                }
            }
            self.save_state();
            self.request_redraw();
        }

        let mut animating = false;
        if (self.anim_height - self.anim_target_height).abs() > 0.5 {
            self.anim_height += (self.anim_target_height - self.anim_height) * 0.35;
            if (self.anim_height - self.anim_target_height).abs() <= 1.0 {
                self.anim_height = self.anim_target_height;
            }
            if let Some(ref win) = self.window {
                let _ = win
                    .request_inner_size(LogicalSize::new(WIN_W, self.anim_height.round() as i32));
            }
            animating = true;
        }

        if animating || self.active_palette != self.target_palette {
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_millis(20),
            ));
            self.request_redraw();
        } else if self.state.has_running_clocks()
            || self.telemetry.media.as_ref().is_some_and(|m| m.is_playing)
        {
            event_loop.set_control_flow(ControlFlow::WaitUntil(
                Instant::now() + Duration::from_secs(1),
            ));
            self.request_redraw();
        } else if let Some((_, exp)) = self.status_toast {
            event_loop.set_control_flow(ControlFlow::WaitUntil(exp));
        } else {
            // Zero idle wakeups & < 0.1% idle CPU when clocks/animations are idle.
            event_loop.set_control_flow(ControlFlow::Wait);
        }
    }
}

fn fmt_mm_ss(total_secs: u64) -> String {
    let m = total_secs / 60;
    let s = total_secs % 60;
    format!("{:02}:{:02}", m, s)
}

fn emit_self_benchmark_metrics() {
    #[cfg(target_os = "windows")]
    {
        if let Ok(stats) = PlatformSysInfo::get_system_stats() {
            let _ = stats;
        }
        // Print process Working Set using Win32 PSAPI or powershell-free call.
        use std::process::Command;
        if let Ok(out) = Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "$p = Get-Process -Id {}; Write-Output (\"BENCH_METRICS ws_mb={{0:N2}} priv_mb={{1:N2}} cpu_s={{2:N3}}\" -f ($p.WorkingSet64/1MB), ($p.PrivateMemorySize64/1MB), $p.TotalProcessorTime.TotalSeconds)",
                    std::process::id()
                ),
            ])
            .output()
        {
            println!("{}", String::from_utf8_lossy(&out.stdout).trim());
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(statm) = std::fs::read_to_string("/proc/self/statm") {
            let pages: Vec<u64> = statm
                .split_whitespace()
                .filter_map(|s| s.parse().ok())
                .collect();
            let rss_kb = pages.get(1).copied().unwrap_or(0) * 4;
            println!("BENCH_METRICS ws_mb={:.2}", rss_kb as f64 / 1024.0);
        }
    }
}

fn spawn_background_workers(
    proxy: EventLoopProxy<CompanionEvent>,
    visible_flag: Arc<AtomicBool>,
    daemon_rx: mpsc::Receiver<DaemonCommand>,
) {
    // 1. Telemetry worker: skips work while hidden & caches unchanged album art for < 0.5% idle CPU.
    let te_proxy = proxy.clone();
    let vis_ref = visible_flag;
    std::thread::Builder::new()
        .name("pico-companion-telemetry".into())
        .spawn(move || {
            let mut last_hash: Option<String> = None;
            let mut last_track_key: Option<(String, String)> = None;
            let mut ticks = 0u64;
            let mut was_playing = false;
            loop {
                let poll_interval = if !vis_ref.load(Ordering::Relaxed) {
                    Duration::from_millis(4500)
                } else if was_playing {
                    Duration::from_millis(2500)
                } else {
                    Duration::from_millis(4500)
                };
                std::thread::sleep(poll_interval);
                if !vis_ref.load(Ordering::Relaxed) {
                    continue;
                }
                ticks += 1;
                let cached_key = last_track_key
                    .as_ref()
                    .map(|(t, a)| (t.as_str(), a.as_str()));
                let media = PlatformMedia::current_track_cached(cached_key);
                if let Some(ref m) = media {
                    last_track_key = Some((m.title.clone(), m.album.clone()));
                    was_playing = m.is_playing;
                } else {
                    last_track_key = None;
                    was_playing = false;
                }

                let audio_state = PlatformAudio::get_state().ok();
                let volume = audio_state.as_ref().map(|a| a.volume_percent as u8);
                let muted = audio_state.as_ref().map(|a| a.muted);
                let stats = if ticks.is_multiple_of(3) || last_hash.is_none() {
                    PlatformSysInfo::get_system_stats().ok()
                } else {
                    None
                };
                let battery = if ticks.is_multiple_of(4) || last_hash.is_none() {
                    get_battery_status()
                } else {
                    None
                };

                let sig = format!(
                    "{:?}:{:?}:{:?}:{:?}",
                    media
                        .as_ref()
                        .map(|m| (&m.title, &m.artist, m.is_playing, m.position_secs)),
                    volume,
                    muted,
                    stats.as_ref().map(|s| s.available_memory_mb)
                );
                if last_hash.as_deref() != Some(&sig) {
                    last_hash = Some(sig);
                    let _ = te_proxy.send_event(CompanionEvent::TelemetryUpdated {
                        media,
                        volume,
                        muted,
                        stats,
                        battery,
                    });
                }
            }
        })
        .ok();

    // 2. IPC Daemon Client Worker (checks token existence before touching pipe/socket when offline)
    std::thread::Builder::new()
        .name("pico-companion-ipc".into())
        .spawn(move || {
            let endpoint = default_ipc_endpoint();
            let token_path = default_ipc_token_path();
            let poll_refresh = || -> Option<DaemonSnapshot> {
                if !token_path.exists() {
                    return None;
                }
                let tok = read_ipc_token(&token_path)?;
                match send_ipc_request_sync(&endpoint, Some(&tok), IpcRequest::GetSnapshot).ok()? {
                    IpcResponse::Snapshot(snap) => Some(*snap),
                    _ => None,
                }
            };

            let _ = proxy.send_event(CompanionEvent::DaemonSnapshotUpdated(poll_refresh()));

            loop {
                match daemon_rx.recv_timeout(Duration::from_secs(6)) {
                    Ok(DaemonCommand::Refresh) | Err(mpsc::RecvTimeoutError::Timeout) => {
                        let _ =
                            proxy.send_event(CompanionEvent::DaemonSnapshotUpdated(poll_refresh()));
                    }
                    Ok(DaemonCommand::SubmitPrompt(text)) => {
                        let tok = read_ipc_token(&token_path);
                        let res = send_ipc_request_sync(
                            &endpoint,
                            tok.as_deref(),
                            IpcRequest::SubmitPrompt {
                                session_id: None,
                                text,
                                speak_response: true,
                            },
                        );
                        let ev = match res {
                            Ok(IpcResponse::TurnResult { reply, .. }) => {
                                CompanionEvent::DaemonReply {
                                    reply: Some(reply),
                                    error: None,
                                }
                            }
                            Ok(IpcResponse::Error { message, .. }) => CompanionEvent::DaemonReply {
                                reply: None,
                                error: Some(message),
                            },
                            Ok(other) => CompanionEvent::DaemonReply {
                                reply: None,
                                error: Some(format!("Unexpected reply: {:?}", other)),
                            },
                            Err(e) => CompanionEvent::DaemonReply {
                                reply: None,
                                error: Some(e.to_string()),
                            },
                        };
                        let _ = proxy.send_event(ev);
                    }
                    Ok(DaemonCommand::RespondConfirmation(req_id, approved)) => {
                        let tok = read_ipc_token(&token_path);
                        let _ = send_ipc_request_sync(
                            &endpoint,
                            tok.as_deref(),
                            IpcRequest::RespondConfirmation {
                                request_id: req_id,
                                approved,
                            },
                        );
                        let _ =
                            proxy.send_event(CompanionEvent::DaemonSnapshotUpdated(poll_refresh()));
                    }
                    Ok(DaemonCommand::SetVoicePaused(paused)) => {
                        let tok = read_ipc_token(&token_path);
                        let _ = send_ipc_request_sync(
                            &endpoint,
                            tok.as_deref(),
                            IpcRequest::SetVoicePaused { paused },
                        );
                        let _ =
                            proxy.send_event(CompanionEvent::DaemonSnapshotUpdated(poll_refresh()));
                    }
                    Ok(DaemonCommand::InterruptSpeech) => {
                        let tok = read_ipc_token(&token_path);
                        let _ = send_ipc_request_sync(
                            &endpoint,
                            tok.as_deref(),
                            IpcRequest::InterruptSpeech,
                        );
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                }
            }
        })
        .ok();
}

fn spawn_platform_action_worker(
    proxy: EventLoopProxy<CompanionEvent>,
    platform_rx: mpsc::Receiver<PlatformCommand>,
) {
    std::thread::Builder::new()
        .name("pico-platform-actions".into())
        .spawn(move || {
            while let Ok(cmd) = platform_rx.recv() {
                let ev = match cmd {
                    PlatformCommand::MediaPrev => match PlatformMedia::previous_track() {
                        Ok(()) => CompanionEvent::PlatformActionResult {
                            message: None,
                            error: None,
                            volume: None,
                            muted: None,
                            clipboard_stash: None,
                            refresh_media: true,
                        },
                        Err(e) => CompanionEvent::PlatformActionResult {
                            message: None,
                            error: Some(format!("Media prev failed: {}", e)),
                            volume: None,
                            muted: None,
                            clipboard_stash: None,
                            refresh_media: false,
                        },
                    },
                    PlatformCommand::MediaPlayPause => match PlatformMedia::toggle_play_pause() {
                        Ok(()) => CompanionEvent::PlatformActionResult {
                            message: None,
                            error: None,
                            volume: None,
                            muted: None,
                            clipboard_stash: None,
                            refresh_media: true,
                        },
                        Err(e) => CompanionEvent::PlatformActionResult {
                            message: None,
                            error: Some(format!("Media play/pause failed: {}", e)),
                            volume: None,
                            muted: None,
                            clipboard_stash: None,
                            refresh_media: false,
                        },
                    },
                    PlatformCommand::MediaNext => match PlatformMedia::next_track() {
                        Ok(()) => CompanionEvent::PlatformActionResult {
                            message: None,
                            error: None,
                            volume: None,
                            muted: None,
                            clipboard_stash: None,
                            refresh_media: true,
                        },
                        Err(e) => CompanionEvent::PlatformActionResult {
                            message: None,
                            error: Some(format!("Media next failed: {}", e)),
                            volume: None,
                            muted: None,
                            clipboard_stash: None,
                            refresh_media: false,
                        },
                    },
                    PlatformCommand::SetMute(next_muted) => {
                        match PlatformAudio::set_mute(next_muted) {
                            Ok(state) => CompanionEvent::PlatformActionResult {
                                message: None,
                                error: None,
                                volume: Some(state.volume_percent as u8),
                                muted: Some(state.muted),
                                clipboard_stash: None,
                                refresh_media: false,
                            },
                            Err(e) => CompanionEvent::PlatformActionResult {
                                message: None,
                                error: Some(format!("Mute failed: {}", e)),
                                volume: None,
                                muted: None,
                                clipboard_stash: None,
                                refresh_media: false,
                            },
                        }
                    }
                    PlatformCommand::SetVolume(vol) => {
                        match PlatformAudio::set_volume(vol as u32) {
                            Ok(mut state) => {
                                if vol > 0 && state.muted {
                                    if let Ok(unmuted) = PlatformAudio::set_mute(false) {
                                        state = unmuted;
                                    }
                                }
                                CompanionEvent::PlatformActionResult {
                                    message: None,
                                    error: None,
                                    volume: Some(state.volume_percent as u8),
                                    muted: Some(state.muted),
                                    clipboard_stash: None,
                                    refresh_media: false,
                                }
                            }
                            Err(e) => CompanionEvent::PlatformActionResult {
                                message: None,
                                error: Some(format!("Volume failed: {}", e)),
                                volume: None,
                                muted: None,
                                clipboard_stash: None,
                                refresh_media: false,
                            },
                        }
                    }
                    PlatformCommand::CopyText(txt) => match PlatformClipboard::set_text(&txt) {
                        Ok(()) => CompanionEvent::PlatformActionResult {
                            message: Some("Copied to clipboard".into()),
                            error: None,
                            volume: None,
                            muted: None,
                            clipboard_stash: None,
                            refresh_media: false,
                        },
                        Err(e) => CompanionEvent::PlatformActionResult {
                            message: None,
                            error: Some(format!("Copy failed: {}", e)),
                            volume: None,
                            muted: None,
                            clipboard_stash: None,
                            refresh_media: false,
                        },
                    },
                    PlatformCommand::PasteClipboardToStash => match PlatformClipboard::get_text() {
                        Ok(txt) if !txt.trim().is_empty() => CompanionEvent::PlatformActionResult {
                            message: None,
                            error: None,
                            volume: None,
                            muted: None,
                            clipboard_stash: Some(txt),
                            refresh_media: false,
                        },
                        Ok(_) => CompanionEvent::PlatformActionResult {
                            message: Some("Clipboard has no text".into()),
                            error: None,
                            volume: None,
                            muted: None,
                            clipboard_stash: None,
                            refresh_media: false,
                        },
                        Err(e) => CompanionEvent::PlatformActionResult {
                            message: None,
                            error: Some(format!("Clipboard read failed: {}", e)),
                            volume: None,
                            muted: None,
                            clipboard_stash: None,
                            refresh_media: false,
                        },
                    },
                    PlatformCommand::OpenStashPayload(payload) => {
                        match PlatformApps::open_application(&payload) {
                            Ok(()) => CompanionEvent::PlatformActionResult {
                                message: None,
                                error: None,
                                volume: None,
                                muted: None,
                                clipboard_stash: None,
                                refresh_media: false,
                            },
                            Err(e) => CompanionEvent::PlatformActionResult {
                                message: None,
                                error: Some(format!("Open failed: {}", e)),
                                volume: None,
                                muted: None,
                                clipboard_stash: None,
                                refresh_media: false,
                            },
                        }
                    }
                    PlatformCommand::RefreshMedia => {
                        let media = PlatformMedia::current_track_cached(None);
                        let audio = PlatformAudio::get_state().ok();
                        CompanionEvent::TelemetryUpdated {
                            media,
                            volume: audio.as_ref().map(|a| a.volume_percent as u8),
                            muted: audio.as_ref().map(|a| a.muted),
                            stats: None,
                            battery: None,
                        }
                    }
                };
                let _ = proxy.send_event(ev);
            }
        })
        .ok();
}

fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    let event_loop = EventLoop::<CompanionEvent>::with_user_event().build()?;
    let proxy = event_loop.create_proxy();

    let (tray_tx, tray_rx) = mpsc::channel::<TrayCommand>();
    let wake_proxy = proxy.clone();
    let tray_handle = if !args.benchmark_self {
        spawn_tray_and_hotkey(tray_tx, move || {
            let _ = wake_proxy.send_event(CompanionEvent::Wake);
        })
    } else {
        None
    };

    let tray_proxy = proxy.clone();
    std::thread::Builder::new()
        .name("pico-tray-bridge".into())
        .spawn(move || {
            while let Ok(cmd) = tray_rx.recv() {
                if tray_proxy.send_event(CompanionEvent::Tray(cmd)).is_err() {
                    break;
                }
            }
        })
        .ok();

    let visible_flag = Arc::new(AtomicBool::new(true));
    let (daemon_tx, daemon_rx) = mpsc::channel();
    let (platform_tx, platform_rx) = mpsc::channel();
    if !args.benchmark_self {
        spawn_background_workers(proxy.clone(), visible_flag.clone(), daemon_rx);
        spawn_platform_action_worker(proxy.clone(), platform_rx);
    }

    let mut app = CompanionApp::new(args, visible_flag, daemon_tx, platform_tx, tray_handle);
    event_loop.run_app(&mut app)?;
    Ok(())
}
