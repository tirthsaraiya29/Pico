use pico_core::error::Result;
use serde::{Deserialize, Serialize};

/// Currently playing or paused OS media track snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaTrackInfo {
    pub has_session: bool,
    pub title: String,
    pub artist: String,
    pub album: String,
    pub source_app: String,
    pub is_playing: bool,
    pub position_secs: u64,
    pub duration_secs: u64,
    pub thumbnail_bytes: Option<Vec<u8>>,
}

/// Cross-platform OS media session reader and playback controller.
/// Uses WinRT `GlobalSystemMediaTransportControlsSessionManager` on Windows 11
/// and MPRIS2 (`playerctl`) on Linux.
pub struct PlatformMedia;

impl PlatformMedia {
    pub fn current_track() -> Option<MediaTrackInfo> {
        Self::current_track_cached(None)
    }

    /// Reads current OS media metadata; only fetches album thumbnail bytes when `(title, album)` differs
    /// from `cached_title_album` so periodic refreshes avoid repeated image stream transfers.
    pub fn current_track_cached(
        cached_title_album: Option<(&str, &str)>,
    ) -> Option<MediaTrackInfo> {
        #[cfg(target_os = "windows")]
        {
            winrt_media::current_track(cached_title_album)
        }
        #[cfg(target_os = "linux")]
        {
            linux_mpris::current_track(cached_title_album)
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            let _ = cached_title_album;
            None
        }
    }

    pub fn toggle_play_pause() -> Result<()> {
        #[cfg(target_os = "windows")]
        {
            winrt_media::control("play_pause")
        }
        #[cfg(target_os = "linux")]
        {
            linux_mpris::control("play-pause")
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            Err(pico_core::error::PicoError::Platform(
                "Unsupported OS for media control".into(),
            ))
        }
    }

    pub fn next_track() -> Result<()> {
        #[cfg(target_os = "windows")]
        {
            winrt_media::control("next")
        }
        #[cfg(target_os = "linux")]
        {
            linux_mpris::control("next")
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            Err(pico_core::error::PicoError::Platform(
                "Unsupported OS for media control".into(),
            ))
        }
    }

    pub fn previous_track() -> Result<()> {
        #[cfg(target_os = "windows")]
        {
            winrt_media::control("previous")
        }
        #[cfg(target_os = "linux")]
        {
            linux_mpris::control("previous")
        }
        #[cfg(not(any(target_os = "windows", target_os = "linux")))]
        {
            Err(pico_core::error::PicoError::Platform(
                "Unsupported OS for media control".into(),
            ))
        }
    }
}

#[cfg(target_os = "windows")]
mod winrt_media {
    use super::MediaTrackInfo;
    use pico_core::error::{PicoError, Result};
    use windows::Media::Control::{
        GlobalSystemMediaTransportControlsSessionManager,
        GlobalSystemMediaTransportControlsSessionPlaybackStatus,
    };
    use windows::Storage::Streams::{DataReader, IRandomAccessStreamReference};
    use windows::Win32::System::WinRT::{RoInitialize, RO_INIT_MULTITHREADED};

    fn ensure_winrt() {
        // Safe to call per-thread; ignores already-initialized or STA-mode returns gracefully.
        unsafe {
            let _ = RoInitialize(RO_INIT_MULTITHREADED);
        }
    }

    fn read_thumbnail_bytes(thumb_ref: &IRandomAccessStreamReference) -> Option<Vec<u8>> {
        let stream = thumb_ref.OpenReadAsync().ok()?.get().ok()?;
        let size = stream.Size().ok()?;
        if size == 0 || size > 4 * 1024 * 1024 {
            return None;
        }
        let len = size as u32;
        let input = stream.GetInputStreamAt(0).ok()?;
        let reader = DataReader::CreateDataReader(&input).ok()?;
        let loaded = reader.LoadAsync(len).ok()?.get().ok()?;
        let read_len = loaded.min(len) as usize;
        let mut buf = vec![0u8; read_len];
        reader.ReadBytes(&mut buf).ok()?;
        Some(buf)
    }

    pub fn current_track(cached_title_album: Option<(&str, &str)>) -> Option<MediaTrackInfo> {
        ensure_winrt();
        let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
            .ok()?
            .get()
            .ok()?;
        let session = manager.GetCurrentSession().ok()?;

        let source_app = session
            .SourceAppUserModelId()
            .map(|h| h.to_string_lossy())
            .unwrap_or_else(|_| "Media".into());

        let is_playing = session
            .GetPlaybackInfo()
            .and_then(|info| info.PlaybackStatus())
            .map(|st| st == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing)
            .unwrap_or(false);

        let (position_secs, duration_secs) = session
            .GetTimelineProperties()
            .map(|tl| {
                let pos = tl
                    .Position()
                    .map(|t| (t.Duration / 10_000_000).max(0) as u64)
                    .unwrap_or(0);
                let end = tl
                    .EndTime()
                    .map(|t| (t.Duration / 10_000_000).max(0) as u64)
                    .unwrap_or(0);
                (pos, end)
            })
            .unwrap_or((0, 0));

        let props = session.TryGetMediaPropertiesAsync().ok()?.get().ok()?;
        let title = props
            .Title()
            .map(|h| h.to_string_lossy())
            .unwrap_or_default()
            .trim()
            .to_string();
        let artist = props
            .Artist()
            .map(|h| h.to_string_lossy())
            .unwrap_or_default()
            .trim()
            .to_string();
        let album = props
            .AlbumTitle()
            .map(|h| h.to_string_lossy())
            .unwrap_or_default()
            .trim()
            .to_string();

        if title.is_empty() && artist.is_empty() {
            return None;
        }

        let same_track = cached_title_album.is_some_and(|(ct, ca)| ct == title && ca == album);
        let thumbnail_bytes = if same_track {
            None
        } else {
            props
                .Thumbnail()
                .ok()
                .and_then(|t| read_thumbnail_bytes(&t))
        };

        Some(MediaTrackInfo {
            has_session: true,
            title,
            artist,
            album,
            source_app,
            is_playing,
            position_secs,
            duration_secs,
            thumbnail_bytes,
        })
    }

    pub fn control(action: &str) -> Result<()> {
        ensure_winrt();
        let manager = GlobalSystemMediaTransportControlsSessionManager::RequestAsync()
            .map_err(|e| PicoError::Platform(format!("Media manager unavailable: {}", e)))?
            .get()
            .map_err(|e| PicoError::Platform(format!("Media manager wait failed: {}", e)))?;
        let session = manager
            .GetCurrentSession()
            .map_err(|_| PicoError::NotFound("No active media session found".into()))?;

        let ok = match action {
            "play_pause" => session
                .TryTogglePlayPauseAsync()
                .map_err(|e| PicoError::Platform(e.to_string()))?
                .get()
                .map_err(|e| PicoError::Platform(e.to_string()))?,
            "next" => session
                .TrySkipNextAsync()
                .map_err(|e| PicoError::Platform(e.to_string()))?
                .get()
                .map_err(|e| PicoError::Platform(e.to_string()))?,
            "previous" => session
                .TrySkipPreviousAsync()
                .map_err(|e| PicoError::Platform(e.to_string()))?
                .get()
                .map_err(|e| PicoError::Platform(e.to_string()))?,
            _ => false,
        };

        if ok {
            Ok(())
        } else {
            Err(PicoError::Platform(format!(
                "Media player rejected '{}' command",
                action
            )))
        }
    }
}

#[cfg(target_os = "linux")]
mod linux_mpris {
    use super::MediaTrackInfo;
    use pico_core::error::{PicoError, Result};
    use std::path::Path;
    use std::process::Command;

    pub fn current_track(cached_title_album: Option<(&str, &str)>) -> Option<MediaTrackInfo> {
        let out = Command::new("playerctl")
            .args([
                "metadata",
                "--format",
                "{{status}}\t{{playerName}}\t{{title}}\t{{artist}}\t{{album}}\t{{mpris:artUrl}}\t{{position}}\t{{mpris:length}}",
            ])
            .output()
            .ok()?;
        if !out.status.success() {
            return None;
        }
        let line = String::from_utf8_lossy(&out.stdout);
        let cols: Vec<&str> = line.trim().split('\t').collect();
        if cols.len() < 5 {
            return None;
        }
        let is_playing = cols[0].eq_ignore_ascii_case("playing");
        let source_app = cols[1].trim().to_string();
        let title = cols[2].trim().to_string();
        let artist = cols[3].trim().to_string();
        let album = cols[4].trim().to_string();
        if title.is_empty() && artist.is_empty() {
            return None;
        }
        let same_track = cached_title_album.is_some_and(|(ct, ca)| ct == title && ca == album);
        let art_url = cols.get(5).copied().unwrap_or("");
        let thumbnail_bytes = if same_track {
            None
        } else {
            art_url.strip_prefix("file://").and_then(|raw_path| {
                let decoded = raw_path.replace("%20", " ");
                let p = Path::new(&decoded);
                if p.is_file() {
                    std::fs::read(p).ok().filter(|b| b.len() <= 4 * 1024 * 1024)
                } else {
                    None
                }
            })
        };
        let pos_us: u64 = cols.get(6).and_then(|s| s.parse().ok()).unwrap_or(0);
        let len_us: u64 = cols.get(7).and_then(|s| s.parse().ok()).unwrap_or(0);

        Some(MediaTrackInfo {
            has_session: true,
            title,
            artist,
            album,
            source_app,
            is_playing,
            position_secs: pos_us / 1_000_000,
            duration_secs: len_us / 1_000_000,
            thumbnail_bytes,
        })
    }

    pub fn control(action: &str) -> Result<()> {
        let out = Command::new("playerctl")
            .arg(action)
            .output()
            .map_err(|e| PicoError::NotFound(format!("playerctl unavailable: {}", e)))?;
        if out.status.success() {
            Ok(())
        } else {
            Err(PicoError::Platform(
                String::from_utf8_lossy(&out.stderr).trim().to_string(),
            ))
        }
    }
}
