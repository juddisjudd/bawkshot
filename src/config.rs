use std::fs;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::geom::RectI;
use crate::theme::Mode;

/// What a hotkey or a tray menu entry captures.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CaptureMode {
    Region,
    Fullscreen,
    Monitor,
    Window,
    LastRegion,
}

impl CaptureMode {
    pub const ALL: [CaptureMode; 5] = [
        CaptureMode::Region,
        CaptureMode::Fullscreen,
        CaptureMode::Monitor,
        CaptureMode::Window,
        CaptureMode::LastRegion,
    ];

    pub fn label(self) -> &'static str {
        match self {
            CaptureMode::Region => "region",
            CaptureMode::Fullscreen => "fullscreen",
            CaptureMode::Monitor => "active monitor",
            CaptureMode::Window => "active window",
            CaptureMode::LastRegion => "last region",
        }
    }

    /// The mode named on the command line, such as `last-region`.
    pub fn from_arg(arg: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|mode| mode.label().replace(' ', "-") == arg)
    }

    pub fn hint(self) -> &'static str {
        match self {
            CaptureMode::Region => "drag an area, or click a window to take all of it",
            CaptureMode::Fullscreen => "every monitor in one image",
            CaptureMode::Monitor => "the monitor the pointer is on",
            CaptureMode::Window => "the window that has focus",
            CaptureMode::LastRegion => "the same area as the last region capture",
        }
    }
}

/// What a screen recording is saved as.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecordFormat {
    Video,
    Gif,
}

impl RecordFormat {
    pub const ALL: [RecordFormat; 2] = [RecordFormat::Video, RecordFormat::Gif];

    pub fn label(self) -> &'static str {
        match self {
            RecordFormat::Video => "video",
            RecordFormat::Gif => "gif",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            RecordFormat::Video => "mp4",
            RecordFormat::Gif => "gif",
        }
    }

    pub fn from_arg(arg: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|format| format.label() == arg)
    }
}

/// Anything a global hotkey can start.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Trigger {
    Capture(CaptureMode),
    Record(RecordFormat),
}

impl Trigger {
    pub const ALL: [Trigger; 7] = [
        Trigger::Capture(CaptureMode::Region),
        Trigger::Capture(CaptureMode::Fullscreen),
        Trigger::Capture(CaptureMode::Monitor),
        Trigger::Capture(CaptureMode::Window),
        Trigger::Capture(CaptureMode::LastRegion),
        Trigger::Record(RecordFormat::Video),
        Trigger::Record(RecordFormat::Gif),
    ];

    pub fn label(self) -> &'static str {
        match self {
            Trigger::Capture(mode) => mode.label(),
            Trigger::Record(RecordFormat::Video) => "record video",
            Trigger::Record(RecordFormat::Gif) => "record gif",
        }
    }

    pub fn hint(self) -> &'static str {
        match self {
            Trigger::Capture(mode) => mode.hint(),
            Trigger::Record(RecordFormat::Video) => {
                "an mp4. drag an area, or click a window to follow it. the same keys stop it"
            }
            Trigger::Record(RecordFormat::Gif) => {
                "a gif. drag an area, or click a window to follow it. the same keys stop it"
            }
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Hotkeys {
    pub region: String,
    pub fullscreen: String,
    pub monitor: String,
    pub window: String,
    pub last_region: String,
    pub record_video: String,
    pub record_gif: String,
}

impl Default for Hotkeys {
    fn default() -> Self {
        Self {
            region: "ctrl+printscreen".into(),
            fullscreen: "printscreen".into(),
            monitor: String::new(),
            window: "alt+printscreen".into(),
            last_region: String::new(),
            record_video: "shift+printscreen".into(),
            record_gif: "ctrl+shift+printscreen".into(),
        }
    }
}

impl Hotkeys {
    pub fn get(&self, trigger: Trigger) -> &str {
        match trigger {
            Trigger::Capture(CaptureMode::Region) => &self.region,
            Trigger::Capture(CaptureMode::Fullscreen) => &self.fullscreen,
            Trigger::Capture(CaptureMode::Monitor) => &self.monitor,
            Trigger::Capture(CaptureMode::Window) => &self.window,
            Trigger::Capture(CaptureMode::LastRegion) => &self.last_region,
            Trigger::Record(RecordFormat::Video) => &self.record_video,
            Trigger::Record(RecordFormat::Gif) => &self.record_gif,
        }
    }

    pub fn set(&mut self, trigger: Trigger, keys: String) {
        *match trigger {
            Trigger::Capture(CaptureMode::Region) => &mut self.region,
            Trigger::Capture(CaptureMode::Fullscreen) => &mut self.fullscreen,
            Trigger::Capture(CaptureMode::Monitor) => &mut self.monitor,
            Trigger::Capture(CaptureMode::Window) => &mut self.window,
            Trigger::Capture(CaptureMode::LastRegion) => &mut self.last_region,
            Trigger::Record(RecordFormat::Video) => &mut self.record_video,
            Trigger::Record(RecordFormat::Gif) => &mut self.record_gif,
        } = keys;
    }
}

pub const VIDEO_RATES: [u32; 3] = [15, 30, 60];
pub const GIF_RATES: [u32; 3] = [10, 15, 24];

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub save_dir: PathBuf,
    pub mode: Mode,
    pub open_editor: bool,
    pub copy_to_clipboard: bool,
    pub save_to_disk: bool,
    pub capture_cursor: bool,
    pub hotkeys: Hotkeys,
    pub last_region: Option<RectI>,
    /// Empty to look for ffmpeg in the usual places.
    pub ffmpeg_path: String,
    pub video_fps: u32,
    pub gif_fps: u32,
    pub record_cursor: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            save_dir: default_save_dir(),
            mode: Mode::Dark,
            open_editor: true,
            copy_to_clipboard: true,
            save_to_disk: true,
            capture_cursor: false,
            hotkeys: Hotkeys::default(),
            last_region: None,
            ffmpeg_path: String::new(),
            video_fps: 30,
            gif_fps: 15,
            record_cursor: true,
        }
    }
}

fn default_save_dir() -> PathBuf {
    dirs::picture_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_default()
        .join("bawkshot")
}

/// `BAWKSHOT_HOME` moves settings elsewhere, so a second copy or a test run leaves the real ones alone.
pub fn data_dir() -> Option<PathBuf> {
    if let Some(home) = std::env::var_os("BAWKSHOT_HOME") {
        return Some(PathBuf::from(home));
    }
    Some(dirs::config_dir()?.join("bawkshot"))
}

fn config_path() -> Option<PathBuf> {
    Some(data_dir()?.join("config.json"))
}

impl Config {
    pub fn load() -> Self {
        config_path()
            .and_then(|path| fs::read(path).ok())
            .and_then(|bytes| serde_json::from_slice(&bytes).ok())
            .unwrap_or_default()
    }

    pub fn save(&self) -> std::io::Result<()> {
        let Some(path) = config_path() else {
            return Ok(());
        };
        if let Some(dir) = path.parent() {
            fs::create_dir_all(dir)?;
        }
        let json = serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?;
        fs::write(path, json)
    }

    /// A fresh file name in the save folder, stamped with the local time.
    pub fn next_file(&self, extension: &str) -> PathBuf {
        let stamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S");
        let mut path = self.save_dir.join(format!("bawkshot_{stamp}.{extension}"));
        let mut copy = 2;
        while path.exists() {
            path = self
                .save_dir
                .join(format!("bawkshot_{stamp}_{copy}.{extension}"));
            copy += 1;
        }
        path
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn default_hotkeys_do_not_collide() {
        let hotkeys = Hotkeys::default();
        let set: Vec<&str> = Trigger::ALL
            .into_iter()
            .map(|trigger| hotkeys.get(trigger))
            .filter(|keys| !keys.is_empty())
            .collect();
        assert_eq!(set.iter().collect::<HashSet<_>>().len(), set.len());
    }

    #[test]
    fn every_trigger_reads_back_what_was_set() {
        let mut hotkeys = Hotkeys::default();
        for (index, trigger) in Trigger::ALL.into_iter().enumerate() {
            hotkeys.set(trigger, format!("ctrl+f{}", index + 1));
        }
        for (index, trigger) in Trigger::ALL.into_iter().enumerate() {
            assert_eq!(hotkeys.get(trigger), format!("ctrl+f{}", index + 1));
        }
    }

    #[test]
    fn command_line_names_resolve() {
        assert_eq!(
            CaptureMode::from_arg("last-region"),
            Some(CaptureMode::LastRegion)
        );
        assert_eq!(RecordFormat::from_arg("gif"), Some(RecordFormat::Gif));
        assert_eq!(RecordFormat::from_arg("webm"), None);
    }

    #[test]
    fn an_old_config_file_gains_the_new_settings() {
        let config: Config =
            serde_json::from_str(r#"{"open_editor":false,"hotkeys":{"region":"f9"}}"#).unwrap();
        assert!(!config.open_editor);
        assert_eq!(config.hotkeys.region, "f9");
        assert_eq!(config.hotkeys.record_gif, "ctrl+shift+printscreen");
        assert_eq!(config.video_fps, 30);
    }
}
