use std::fs::{self, File};
use std::io::Write;
use std::os::windows::process::CommandExt as _;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crate::config::RecordFormat;
use crate::geom::RectI;
use crate::platform::{Grabber, Job, WindowState, window_state};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const STOP_TIMEOUT: Duration = Duration::from_secs(15);
const GIF_FILTER: &str = "split[a][b];[a]palettegen=stats_mode=diff[p];[b][p]paletteuse=dither=bayer:bayer_scale=5:diff_mode=rectangle";

/// Where ffmpeg may live, best guess first. A path set by hand is the only place looked.
fn candidates(custom: &str) -> Vec<PathBuf> {
    let custom = custom.trim();
    if !custom.is_empty() {
        return vec![PathBuf::from(custom)];
    }
    let mut folders = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        folders.extend(exe.parent().map(Path::to_path_buf));
    }
    folders.extend(crate::config::data_dir());
    if let Some(path) = std::env::var_os("PATH") {
        folders.extend(std::env::split_paths(&path));
    }
    if let Some(programs) = std::env::var_os("ProgramFiles") {
        folders.push(PathBuf::from(programs).join("ShareX"));
    }
    if let Some(documents) = dirs::document_dir() {
        folders.push(documents.join("ShareX").join("Tools"));
    }
    folders
        .into_iter()
        .map(|folder| folder.join("ffmpeg.exe"))
        .collect()
}

/// A scoop shim starts the real program as a child of its own; running the real one keeps stop and kill direct.
fn unshim(path: PathBuf) -> PathBuf {
    let Ok(shim) = fs::read_to_string(path.with_extension("shim")) else {
        return path;
    };
    shim.lines()
        .find_map(|line| {
            let target = line.trim().strip_prefix("path")?.trim_start();
            let target = PathBuf::from(target.strip_prefix('=')?.trim().trim_matches('"'));
            target.is_file().then_some(target)
        })
        .unwrap_or(path)
}

pub fn locate(custom: &str) -> Option<PathBuf> {
    candidates(custom)
        .into_iter()
        .find(|path| path.is_file())
        .map(unshim)
}

/// An ffmpeg binary, and which of the encoders the recorder prefers it was built with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ffmpeg {
    pub path: PathBuf,
    x264: bool,
    x264_rgb: bool,
    media_foundation: bool,
}

impl Ffmpeg {
    pub fn probe(path: PathBuf) -> Result<Self, String> {
        let output = Command::new(&path)
            .args(["-hide_banner", "-encoders"])
            .stdin(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|err| format!("could not run {}: {err}", path.display()))?;
        Ok(Self::with_encoders(
            path,
            &String::from_utf8_lossy(&output.stdout),
        ))
    }

    fn with_encoders(path: PathBuf, listing: &str) -> Self {
        let has = |name: &str| {
            listing
                .lines()
                .any(|line| line.split_whitespace().nth(1) == Some(name))
        };
        Self {
            path,
            x264: has("libx264"),
            x264_rgb: has("libx264rgb"),
            media_foundation: has("h264_mf"),
        }
    }

    fn video_encoder(&self) -> &'static [&'static str] {
        if self.x264 {
            &[
                "-c:v",
                "libx264",
                "-preset",
                "veryfast",
                "-crf",
                "23",
                "-pix_fmt",
                "yuv420p",
                "-movflags",
                "+faststart",
            ]
        } else if self.media_foundation {
            &["-c:v", "h264_mf", "-b:v", "10M", "-pix_fmt", "yuv420p"]
        } else {
            &["-c:v", "mpeg4", "-q:v", "4", "-pix_fmt", "yuv420p"]
        }
    }

    /// Lossless, because the gif pass picks its palette from this file.
    fn gif_source_encoder(&self) -> &'static [&'static str] {
        if self.x264_rgb {
            &["-c:v", "libx264rgb", "-preset", "ultrafast", "-qp", "0"]
        } else {
            &["-c:v", "utvideo", "-pix_fmt", "gbrp"]
        }
    }
}

/// H.264 needs even sizes, so an odd edge loses its last pixel.
pub fn even(rect: RectI) -> RectI {
    RectI::new(rect.x, rect.y, rect.w & !1, rect.h & !1)
}

/// The ffmpeg input that reads raw frames from the pipe bawkshot writes them to.
fn pipe_input(size: (i32, i32), fps: u32) -> Vec<String> {
    [
        "-f",
        "rawvideo",
        "-pixel_format",
        "bgr0",
        "-video_size",
        &format!("{}x{}", size.0, size.1),
        "-framerate",
        &fps.to_string(),
        "-i",
        "pipe:0",
    ]
    .map(str::to_string)
    .to_vec()
}

fn last_line(text: &str) -> Option<String> {
    text.lines()
        .rev()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .map(str::to_string)
}

pub enum Next<'a> {
    Frame(&'a [u8]),
    /// Nothing new to show, such as while a followed window is minimized.
    Same,
    End,
}

/// A supply of BGRA frames of one fixed size.
pub trait Frames {
    fn next(&mut self) -> Next<'_>;
}

/// Frames of a screen area. With a window given, the area travels with that window's top-left corner.
pub struct ScreenFrames {
    grabber: Grabber,
    origin: (i32, i32),
    window: Option<isize>,
    cursor: bool,
}

impl ScreenFrames {
    pub fn open(area: RectI, window: Option<isize>, cursor: bool) -> Option<Self> {
        Some(Self {
            grabber: Grabber::new(area.w, area.h)?,
            origin: (area.x, area.y),
            window,
            cursor,
        })
    }
}

impl Frames for ScreenFrames {
    fn next(&mut self) -> Next<'_> {
        if let Some(window) = self.window {
            match window_state(window) {
                WindowState::Gone => return Next::End,
                WindowState::Minimized => return Next::Same,
                WindowState::At(rect) => self.origin = (rect.x, rect.y),
            }
        }
        match self
            .grabber
            .frame(self.origin.0, self.origin.1, self.cursor)
        {
            Some(frame) => Next::Frame(frame),
            None => Next::Same,
        }
    }
}

/// Writes one frame per tick of `fps`, repeating the last one when the source is late, so the video keeps real time.
fn pump(
    mut frames: impl Frames,
    mut pipe: impl Write,
    fps: u32,
    frame_len: usize,
    stop: &AtomicBool,
) {
    let started = Instant::now();
    let mut held = vec![0u8; frame_len];
    let mut written = 0u64;
    while !stop.load(Ordering::Relaxed) {
        let due = (started.elapsed().as_secs_f64() * fps as f64) as u64 + 1;
        if written >= due {
            let next_tick = Duration::from_secs_f64(written as f64 / fps as f64);
            let wait = next_tick.saturating_sub(started.elapsed());
            std::thread::sleep(wait.min(Duration::from_millis(40)));
            continue;
        }
        match frames.next() {
            Next::Frame(frame) if frame.len() == frame_len => held.copy_from_slice(frame),
            Next::Frame(_) | Next::Same => {}
            Next::End => return,
        }
        let behind = (due - written).min(fps as u64);
        for _ in 0..behind {
            if pipe.write_all(&held).is_err() {
                return;
            }
        }
        written += behind;
    }
}

pub enum Health {
    Running,
    /// The source ran out, as when a followed window closes; the recording can be saved.
    Ended,
    Failed(String),
}

/// A running capture: a thread pumps frames into ffmpeg. A gif goes to a lossless temporary file and is encoded when it stops.
pub struct Recording {
    child: Child,
    job: Option<Job>,
    pump: Option<JoinHandle<()>>,
    stop: Arc<AtomicBool>,
    ffmpeg: Ffmpeg,
    format: RecordFormat,
    capture: PathBuf,
    output: PathBuf,
    log: PathBuf,
}

impl Recording {
    /// `open` runs on the recorder's own thread, which is where screen capture handles have to live.
    pub fn start<F: Frames>(
        ffmpeg: Ffmpeg,
        format: RecordFormat,
        size: (i32, i32),
        fps: u32,
        open: impl FnOnce() -> Option<F> + Send + 'static,
        output: PathBuf,
        log: PathBuf,
    ) -> Result<Self, String> {
        for folder in [output.parent(), log.parent()].into_iter().flatten() {
            fs::create_dir_all(folder)
                .map_err(|err| format!("could not create {}: {err}", folder.display()))?;
        }
        let (capture, encoder) = match format {
            RecordFormat::Video => (output.clone(), ffmpeg.video_encoder()),
            RecordFormat::Gif => {
                let stamp = chrono::Local::now().format("%H%M%S%3f");
                let name = format!("bawkshot-{}-{stamp}.mkv", std::process::id());
                (std::env::temp_dir().join(name), ffmpeg.gif_source_encoder())
            }
        };
        let errors = File::create(&log)
            .map_err(|err| format!("could not write {}: {err}", log.display()))?;
        let mut child = Command::new(&ffmpeg.path)
            .args(["-y", "-hide_banner", "-loglevel", "error", "-nostats"])
            .args(pipe_input(size, fps))
            .args(encoder)
            .arg(&capture)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(errors)
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|err| format!("could not start {}: {err}", ffmpeg.path.display()))?;
        let job = Job::new();
        if let Some(job) = &job {
            job.adopt(&child);
        }
        let stop = Arc::new(AtomicBool::new(false));
        let pipe = child.stdin.take();
        let frame_len = size.0.max(0) as usize * size.1.max(0) as usize * 4;
        let stopped = stop.clone();
        let pump = std::thread::spawn(move || {
            if let (Some(frames), Some(pipe)) = (open(), pipe) {
                pump(frames, pipe, fps.max(1), frame_len, &stopped);
            }
        });
        Ok(Self {
            child,
            job,
            pump: Some(pump),
            stop,
            ffmpeg,
            format,
            capture,
            output,
            log,
        })
    }

    fn complaint(&self) -> String {
        fs::read_to_string(&self.log)
            .ok()
            .and_then(|log| last_line(&log))
            .unwrap_or_else(|| "ffmpeg stopped without saying why.".into())
    }

    pub fn health(&mut self) -> Health {
        match self.child.try_wait() {
            Ok(Some(status)) if !status.success() => Health::Failed(self.complaint()),
            Ok(Some(_)) => Health::Ended,
            _ if self.pump.as_ref().is_none_or(JoinHandle::is_finished) => Health::Ended,
            _ => Health::Running,
        }
    }

    fn kill(&mut self) {
        match &self.job {
            Some(job) => job.kill(),
            None => {
                let _ = self.child.kill();
            }
        }
        let _ = self.child.wait();
    }

    /// Ends the frame supply, which closes ffmpeg's input, and waits for ffmpeg to finish its file; true when it ended cleanly.
    fn stop(&mut self) -> bool {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(pump) = self.pump.take() {
            let _ = pump.join();
        }
        let deadline = Instant::now() + STOP_TIMEOUT;
        loop {
            match self.child.try_wait() {
                Ok(Some(status)) => return status.success(),
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                _ => {
                    self.kill();
                    return false;
                }
            }
        }
    }

    fn encode_gif(&self) -> Result<(), String> {
        let output = Command::new(&self.ffmpeg.path)
            .args(["-y", "-hide_banner", "-loglevel", "error", "-i"])
            .arg(&self.capture)
            .args(["-vf", GIF_FILTER, "-loop", "0"])
            .arg(&self.output)
            .stdin(Stdio::null())
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|err| format!("could not run the gif pass: {err}"))?;
        if output.status.success() {
            return Ok(());
        }
        let _ = fs::remove_file(&self.output);
        Err(last_line(&String::from_utf8_lossy(&output.stderr))
            .unwrap_or_else(|| "the gif pass failed.".into()))
    }

    /// Stops the capture. Returns the saved file, or `None` when the recording was thrown away. Blocks until done.
    pub fn finish(mut self, keep: bool) -> Result<Option<PathBuf>, String> {
        let clean = self.stop();
        let captured = fs::metadata(&self.capture).is_ok_and(|file| file.len() > 0);
        if !keep {
            let _ = fs::remove_file(&self.capture);
            return Ok(None);
        }
        if !clean || !captured {
            let _ = fs::remove_file(&self.capture);
            return Err(self.complaint());
        }
        if self.format == RecordFormat::Gif {
            let encoded = self.encode_gif();
            let _ = fs::remove_file(&self.capture);
            encoded?;
        }
        Ok(Some(self.output.clone()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ENCODERS: &str = "Encoders:\n V..... = Video\n ------\n V....D libx264              libx264 H.264\n V....D libx264rgb           libx264 H.264 RGB\n V....D h264_mf              H264 via MediaFoundation\n V.S..D mpeg4                MPEG-4 part 2\n";
    const SIZE: (i32, i32) = (64, 48);
    const ROW: usize = SIZE.0 as usize * 4;

    /// A moving bar on a plain field, so no screen is read. Ends by itself after `limit` frames when one is set.
    struct Pattern {
        frame: Vec<u8>,
        count: u32,
        limit: Option<u32>,
    }

    impl Pattern {
        fn new(limit: Option<u32>) -> Self {
            Self {
                frame: vec![40; SIZE.0 as usize * SIZE.1 as usize * 4],
                count: 0,
                limit,
            }
        }
    }

    impl Frames for Pattern {
        fn next(&mut self) -> Next<'_> {
            if self.limit.is_some_and(|limit| self.count >= limit) {
                return Next::End;
            }
            let column = (self.count as usize * 3) % SIZE.0 as usize;
            for row in self.frame.as_chunks_mut::<ROW>().0 {
                row.fill(40);
                row[column * 4..column * 4 + 4].copy_from_slice(&[0, 200, 255, 0]);
            }
            self.count += 1;
            Next::Frame(&self.frame)
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let folder =
            std::env::temp_dir().join(format!("bawkshot-test-{}-{name}", std::process::id()));
        fs::create_dir_all(&folder).unwrap();
        folder
    }

    fn start(format: RecordFormat, limit: Option<u32>, folder: &Path) -> Option<Recording> {
        let ffmpeg = Ffmpeg::probe(locate("")?).ok()?;
        let output = folder.join(format!("clip.{}", format.extension()));
        let log = folder.join("ffmpeg.log");
        let open = move || Some(Pattern::new(limit));
        Some(Recording::start(ffmpeg, format, SIZE, 20, open, output, log).unwrap())
    }

    #[test]
    fn encoders_are_read_from_the_listing() {
        let full = Ffmpeg::with_encoders("ffmpeg.exe".into(), ENCODERS);
        assert!(full.x264 && full.x264_rgb && full.media_foundation);
        assert_eq!(full.video_encoder()[1], "libx264");
        assert_eq!(full.gif_source_encoder()[1], "libx264rgb");

        let bare = Ffmpeg::with_encoders("ffmpeg.exe".into(), " V.S..D mpeg4   MPEG-4 part 2\n");
        assert_eq!(bare.video_encoder()[1], "mpeg4");
        assert_eq!(bare.gif_source_encoder()[1], "utvideo");
    }

    #[test]
    fn the_pipe_input_describes_the_frames() {
        assert_eq!(
            pipe_input((800, 600), 30).join(" "),
            "-f rawvideo -pixel_format bgr0 -video_size 800x600 -framerate 30 -i pipe:0"
        );
    }

    #[test]
    fn odd_sizes_are_trimmed_to_even() {
        assert_eq!(even(RectI::new(3, 5, 801, 599)), RectI::new(3, 5, 800, 598));
    }

    #[test]
    fn a_path_set_by_hand_is_the_only_candidate() {
        assert_eq!(
            candidates(" C:\\tools\\ffmpeg.exe "),
            [PathBuf::from("C:\\tools\\ffmpeg.exe")]
        );
        assert!(candidates("").len() > 1);
    }

    #[test]
    fn a_scoop_shim_resolves_to_the_real_program() {
        let folder = scratch("shim");
        let real = folder.join("real.exe");
        fs::write(&real, b"").unwrap();
        let shim = folder.join("ffmpeg.exe");
        fs::write(&shim, b"").unwrap();
        assert_eq!(unshim(shim.clone()), shim);
        fs::write(
            folder.join("ffmpeg.shim"),
            format!("path = \"{}\"\n", real.display()),
        )
        .unwrap();
        assert_eq!(unshim(shim), real);
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn the_pump_keeps_real_time_when_the_source_is_slow() {
        struct Slow(Vec<u8>);
        impl Frames for Slow {
            fn next(&mut self) -> Next<'_> {
                std::thread::sleep(Duration::from_millis(150));
                Next::Frame(&self.0)
            }
        }
        let stop = Arc::new(AtomicBool::new(false));
        let stopper = stop.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(1000));
            stopper.store(true, Ordering::Relaxed);
        });
        let mut written = Vec::new();
        pump(Slow(vec![7; 16]), &mut written, 20, 16, &stop);
        let frames = written.len() / 16;
        assert!(
            (16..=26).contains(&frames),
            "{frames} frames in a second at 20 fps"
        );
        assert!(written.iter().all(|byte| *byte == 7));
    }

    #[test]
    fn the_pump_stops_when_the_source_ends() {
        let mut written = Vec::new();
        let stop = AtomicBool::new(false);
        pump(Pattern::new(Some(3)), &mut written, 200, 64 * 48 * 4, &stop);
        assert_eq!(written.len() / (64 * 48 * 4), 3);
    }

    #[test]
    fn a_video_is_finished_when_stopped() {
        let folder = scratch("video");
        let Some(mut recording) = start(RecordFormat::Video, None, &folder) else {
            return;
        };
        std::thread::sleep(Duration::from_millis(1200));
        assert!(matches!(recording.health(), Health::Running));
        let saved = recording.finish(true).unwrap().expect("a kept recording");
        let bytes = fs::read(saved).unwrap();
        assert!(bytes.len() > 500, "only {} bytes", bytes.len());
        assert!(bytes.windows(4).any(|window| window == b"moov"));
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_gif_is_encoded_when_stopped() {
        let folder = scratch("gif");
        let Some(recording) = start(RecordFormat::Gif, None, &folder) else {
            return;
        };
        std::thread::sleep(Duration::from_millis(1200));
        let saved = recording.finish(true).unwrap().expect("a kept recording");
        assert_eq!(&fs::read(saved).unwrap()[..6], b"GIF89a");
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_discarded_recording_leaves_no_file() {
        let folder = scratch("discard");
        let Some(recording) = start(RecordFormat::Video, None, &folder) else {
            return;
        };
        std::thread::sleep(Duration::from_millis(800));
        assert_eq!(recording.finish(false).unwrap(), None);
        assert!(!folder.join("clip.mp4").exists());
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn a_source_that_ends_leaves_a_recording_to_save() {
        let folder = scratch("ended");
        let Some(mut recording) = start(RecordFormat::Video, Some(12), &folder) else {
            return;
        };
        std::thread::sleep(Duration::from_millis(1800));
        assert!(matches!(recording.health(), Health::Ended));
        assert!(recording.finish(true).unwrap().is_some());
        fs::remove_dir_all(folder).unwrap();
    }

    #[test]
    fn an_output_ffmpeg_cannot_write_is_reported() {
        let folder = scratch("bad");
        fs::create_dir_all(folder.join("clip.mp4")).unwrap();
        let Some(mut recording) = start(RecordFormat::Video, None, &folder) else {
            return;
        };
        std::thread::sleep(Duration::from_millis(1200));
        let Health::Failed(complaint) = recording.health() else {
            panic!("ffmpeg should have quit");
        };
        assert!(!complaint.is_empty());
        assert!(recording.finish(true).is_err());
        fs::remove_dir_all(folder).unwrap();
    }
}
