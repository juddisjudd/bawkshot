#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[cfg(not(windows))]
compile_error!("bawkshot only runs on Windows for now");

mod assets;
mod autostart;
mod config;
mod editor;
mod geom;
mod imaging;
mod logging;
mod overlay;
mod pill;
mod platform;
mod recorder;
mod settings;
mod shell;
mod theme;
mod updater;
mod updates;

use std::time::Duration;

use gpui_kit::*;

use config::{CaptureMode, RecordFormat};
use geom::RectI;
use shell::{Command, RecordTarget};

/// An area given as `x,y,width,height` in screen pixels.
fn parse_area(text: &str) -> Option<RectI> {
    let numbers: Vec<i32> = text
        .split(',')
        .map(|part| part.trim().parse().ok())
        .collect::<Option<_>>()?;
    match numbers.as_slice() {
        [x, y, w, h] if *w > 0 && *h > 0 => Some(RectI::new(*x, *y, *w, *h)),
        _ => None,
    }
}

/// A window to follow: `active`, or a window handle as a decimal or `0x` number.
fn parse_window(text: &str) -> Option<RecordTarget> {
    if text == "active" {
        return Some(RecordTarget::ActiveWindow);
    }
    let handle = match text.strip_prefix("0x") {
        Some(hex) => isize::from_str_radix(hex, 16).ok()?,
        None => text.parse().ok()?,
    };
    (handle != 0).then_some(RecordTarget::Window(handle))
}

/// Reads the launch arguments: `--capture <mode>`, `--record <video|gif>` with optional `--area x,y,w,h` or `--window <active|handle>` and `--seconds n`, `--settings`, or image files to open.
fn startup_commands(args: impl Iterator<Item = String>) -> Vec<Command> {
    let mut commands = Vec::new();
    let mut args = args.peekable();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--settings" => commands.push(Command::Settings),
            "--capture" => {
                if let Some(mode) = args.next().as_deref().and_then(CaptureMode::from_arg) {
                    commands.push(Command::Capture {
                        mode,
                        from_tray: false,
                    });
                }
            }
            "--record" => {
                let Some(format) = args.next().as_deref().and_then(RecordFormat::from_arg) else {
                    continue;
                };
                let (mut target, mut seconds) = (None, None);
                loop {
                    match args.peek().map(String::as_str) {
                        Some("--area") => {
                            args.next();
                            let area = args.next().as_deref().and_then(parse_area);
                            target = area.map(RecordTarget::Area);
                        }
                        Some("--window") => {
                            args.next();
                            target = args.next().as_deref().and_then(parse_window);
                        }
                        Some("--seconds") => {
                            args.next();
                            seconds = args.next().and_then(|text| text.parse().ok());
                        }
                        _ => break,
                    }
                }
                commands.push(Command::Record {
                    format,
                    from_tray: false,
                    target,
                    seconds,
                });
            }
            path if !path.starts_with("--") => commands.push(Command::OpenPath(path.into())),
            _ => {}
        }
    }
    commands
}

fn main() {
    logging::init();
    let after_update = std::env::args().any(|arg| arg == updater::AFTER_UPDATE);
    let patience = Duration::from_secs(if after_update { 30 } else { 0 });
    let Some(_instance) = platform::single_instance(patience) else {
        return;
    };
    updater::clean_up();
    updater::sync_installed_version();
    platform::discard_launch_show_state();
    application()
        .with_assets(assets::AppAssets)
        .with_quit_mode(QuitMode::Explicit)
        .run(|cx| {
            init(cx);
            cx.set_app_identity("com.bawkshot.app", "bawkshot");
            theme::init(cx);
            shell::init(startup_commands(std::env::args().skip(1)), cx);
        });
}

#[cfg(test)]
mod tests {
    use super::{Command, RecordTarget, parse_area, parse_window, startup_commands};
    use crate::config::{CaptureMode, RecordFormat};
    use crate::geom::RectI;

    fn parse(line: &str) -> Vec<Command> {
        startup_commands(line.split_whitespace().map(str::to_string))
    }

    #[test]
    fn areas_need_four_numbers_and_a_real_size() {
        assert_eq!(
            parse_area("-1920, 40,800,600"),
            Some(RectI::new(-1920, 40, 800, 600))
        );
        assert_eq!(parse_area("0,0,800"), None);
        assert_eq!(parse_area("0,0,0,600"), None);
        assert_eq!(parse_area("a,b,c,d"), None);
    }

    #[test]
    fn a_record_command_takes_its_area_and_length() {
        let commands = parse("--record gif --seconds 5 --area 10,20,640,360 shot.png");
        let [
            Command::Record {
                format,
                target,
                seconds,
                ..
            },
            Command::OpenPath(path),
        ] = commands.as_slice()
        else {
            panic!("unexpected commands: {commands:?}");
        };
        assert_eq!(*format, RecordFormat::Gif);
        assert_eq!(
            *target,
            Some(RecordTarget::Area(RectI::new(10, 20, 640, 360)))
        );
        assert_eq!(*seconds, Some(5));
        assert_eq!(path.to_str(), Some("shot.png"));
    }

    #[test]
    fn a_record_command_can_follow_the_active_window() {
        let commands = parse("--record video --window active");
        assert!(matches!(
            commands.as_slice(),
            [Command::Record {
                target: Some(RecordTarget::ActiveWindow),
                seconds: None,
                ..
            }]
        ));
    }

    #[test]
    fn a_window_is_named_by_handle_or_as_the_active_one() {
        assert_eq!(parse_window("active"), Some(RecordTarget::ActiveWindow));
        assert_eq!(parse_window("0x1A2B"), Some(RecordTarget::Window(0x1a2b)));
        assert_eq!(parse_window("6699"), Some(RecordTarget::Window(6699)));
        assert_eq!(parse_window("0"), None);
        assert_eq!(parse_window("notepad"), None);
    }

    #[test]
    fn unknown_names_are_skipped() {
        assert!(parse("--capture nowhere --record webm --bogus").is_empty());
        let commands = parse("--capture last-region --settings");
        assert!(matches!(
            commands.as_slice(),
            [
                Command::Capture {
                    mode: CaptureMode::LastRegion,
                    ..
                },
                Command::Settings
            ]
        ));
    }
}
