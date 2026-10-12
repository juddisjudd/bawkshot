# bawkshot

[![Windows](https://img.shields.io/badge/windows-f6d56b?style=flat-square)](#install)
[![License: MIT](https://img.shields.io/badge/license-MIT-f6d56b?style=flat-square&labelColor=13100f)](LICENSE)

bawkshot is a Windows screenshot tool that lives in the tray. Press a hotkey, pick an area or a window, and mark the capture up before you paste or save it. It also records an area or a window to an MP4 or a GIF.

![The editor with a rectangle, an arrow, a label, step numbers, a highlight, a blur and a pixelated name on a capture](docs/screenshots/editor.png)

## Install

Download the newest `bawkshot-…-windows-x64-setup.exe` from the [releases page](https://github.com/juddisjudd/bawkshot/releases) and run it. You need Windows 10 or 11, 64-bit. It does not ask for an administrator. A zip is there too if you don't want an installer.

Windows may warn that the installer is not signed. Click **More info**, then **Run anyway**.

bawkshot has no main window. Look for its icon in the tray. Right-click it for every capture mode and for **settings**.

## Keys

| Press | To |
| --- | --- |
| `Ctrl+PrtSc` | capture a region, or click a window to take all of it |
| `PrtSc` | capture every monitor |
| `Alt+PrtSc` | capture the window that has focus |
| `Shift+PrtSc` | start or stop a video recording |
| `Ctrl+Shift+PrtSc` | start or stop a GIF recording |

Change them under **settings**. In the editor, press **Enter** to copy, save and close.

Recording needs [ffmpeg](https://ffmpeg.org) on your `PATH`. One way to install it is `winget install ffmpeg`.

## Updates

bawkshot looks for a new version when it starts, and once a day after that. To update, open **settings**, scroll to **about** and press **update**.

## Build

You need Rust 1.92 or later with the MSVC toolchain.

```sh
cargo run --release       # build and start
cargo test                # run the tests
```

bawkshot is written in Rust with [GPUI Kit](https://gpui-kit.com). To publish a release, run `.\scripts\release.ps1 -Version 0.2.0` from a clean `main`.

## License

[MIT](LICENSE).
