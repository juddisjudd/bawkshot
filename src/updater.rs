use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use serde::Deserialize;
use sha2::{Digest, Sha256};
use windows_registry::CURRENT_USER;

pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// Given to the copy an update starts, so it waits for this one to leave instead of bowing out.
pub const AFTER_UPDATE: &str = "--after-update";
const LATEST_API: &str = "https://api.github.com/repos/juddisjudd/bawkshot/releases/latest";
const ASSET_SUFFIX: &str = "-windows-x64.zip";
/// The installer's entry under installed apps, named after the AppId in `packaging/windows/bawkshot.iss`.
const UNINSTALL_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Uninstall\{6F2B8C51-3D0E-4A9B-9C47-B2A5E1D7F083}_is1";

#[derive(Deserialize)]
struct ApiRelease {
    tag_name: String,
    html_url: String,
    assets: Vec<ApiAsset>,
}

#[derive(Deserialize)]
struct ApiAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

/// A published release newer than this build, with its zip and the checksum to verify it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Release {
    pub version: String,
    pub page: String,
    pub size: u64,
    asset_url: String,
    sum_url: String,
}

pub fn parse_version(text: &str) -> Option<(u64, u64, u64)> {
    let mut parts = text.trim().trim_start_matches('v').split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next()?.split(['-', '+']).next()?.parse().ok()?;
    Some((major, minor, patch))
}

pub fn is_newer(candidate: &str, current: &str) -> bool {
    match (parse_version(candidate), parse_version(current)) {
        (Some(candidate), Some(current)) => candidate > current,
        _ => false,
    }
}

fn agent() -> ureq::Agent {
    ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(120)))
        .user_agent(format!("bawkshot/{VERSION}"))
        .build()
        .into()
}

fn pick(release: ApiRelease) -> Result<Release, String> {
    let download = release
        .assets
        .iter()
        .find(|asset| asset.name.ends_with(ASSET_SUFFIX))
        .ok_or("the newest release has no download for windows yet")?;
    let sum_name = format!("{}.sha256", download.name);
    let sum = release
        .assets
        .iter()
        .find(|asset| asset.name == sum_name)
        .ok_or("the newest release has no checksum to verify it with")?;
    Ok(Release {
        version: release.tag_name.trim_start_matches('v').to_string(),
        page: release.html_url,
        size: download.size,
        asset_url: download.browser_download_url.clone(),
        sum_url: sum.browser_download_url.clone(),
    })
}

/// A debug build can be pointed at a stand-in for GitHub, to try an update that was never published.
fn latest_api() -> String {
    #[cfg(debug_assertions)]
    if let Ok(api) = std::env::var("BAWKSHOT_UPDATE_API") {
        return api;
    }
    LATEST_API.into()
}

/// The newest release on GitHub, when it is newer than this build.
pub fn check() -> Result<Option<Release>, String> {
    check_at(&latest_api())
}

fn check_at(api: &str) -> Result<Option<Release>, String> {
    let response = agent()
        .get(api)
        .header("Accept", "application/vnd.github+json")
        .call();
    let release: ApiRelease = match response {
        Err(ureq::Error::StatusCode(404)) => {
            return Err("GitHub lists no published release of bawkshot yet".into());
        }
        Err(err) => return Err(format!("could not reach GitHub: {err}")),
        Ok(mut response) => response
            .body_mut()
            .read_json()
            .map_err(|err| format!("GitHub sent something unexpected: {err}"))?,
    };
    if !is_newer(&release.tag_name, VERSION) {
        return Ok(None);
    }
    pick(release).map(Some)
}

/// The exe to replace, unless this copy was built from source and updates through git.
pub fn install_path() -> Result<PathBuf, String> {
    let exe = std::env::current_exe().map_err(|err| err.to_string())?;
    let from_source = exe
        .parent()
        .and_then(|dir| Some((dir.file_name()?, dir.parent()?.file_name()?)))
        .is_some_and(|(profile, target)| {
            target == "target" && (profile == "debug" || profile == "release")
        });
    if from_source {
        return Err("this copy was built from source, so update it with git and cargo".into());
    }
    Ok(exe)
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

fn sibling(exe: &Path, tag: &str) -> PathBuf {
    let stem = exe.file_stem().unwrap_or_default().to_string_lossy();
    exe.with_file_name(format!("{stem}.{tag}.exe"))
}

/// Downloads and checks the release, then puts the new exe where the running one was. It runs after a restart.
pub fn install(release: &Release, progress: impl Fn(u64)) -> Result<PathBuf, String> {
    let exe = install_path()?;
    install_to(release, &exe, progress).map(|()| exe)
}

fn install_to(release: &Release, exe: &Path, progress: impl Fn(u64)) -> Result<(), String> {
    let agent = agent();
    let sums = agent
        .get(&release.sum_url)
        .call()
        .map_err(|err| format!("could not download the checksum: {err}"))?
        .body_mut()
        .read_to_string()
        .map_err(|err| err.to_string())?;
    let expected = sums
        .split_whitespace()
        .next()
        .ok_or("the checksum file is empty")?
        .to_lowercase();

    static DOWNLOADS: AtomicU32 = AtomicU32::new(0);
    let nth = DOWNLOADS.fetch_add(1, Ordering::Relaxed);
    let name = format!("bawkshot-{}-{nth}.update", std::process::id());
    let download = std::env::temp_dir().join(name);
    let mut response = agent
        .get(&release.asset_url)
        .call()
        .map_err(|err| format!("could not download the update: {err}"))?;
    let mut reader = response.body_mut().as_reader();
    let mut file = File::create(&download).map_err(|err| err.to_string())?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0u8; 64 * 1024];
    let mut done = 0u64;
    loop {
        let read = reader
            .read(&mut buf)
            .map_err(|err| format!("the download broke off: {err}"))?;
        if read == 0 {
            break;
        }
        hasher.update(&buf[..read]);
        file.write_all(&buf[..read])
            .map_err(|err| err.to_string())?;
        done += read as u64;
        progress(done);
    }
    drop(file);
    if hex(&hasher.finalize()) != expected {
        let _ = fs::remove_file(&download);
        return Err("the download does not match its checksum, so it was not installed".into());
    }

    let fresh = sibling(exe, "new");
    let result = extract_exe(&download, &fresh).and_then(|()| swap(exe, &fresh));
    let _ = fs::remove_file(&fresh);
    let _ = fs::remove_file(&download);
    result
}

fn extract_exe(zip_path: &Path, out: &Path) -> Result<(), String> {
    let file = File::open(zip_path).map_err(|err| err.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|err| err.to_string())?;
    let name = (0..archive.len())
        .filter_map(|ix| archive.name_for_index(ix).map(str::to_string))
        .find(|name| name.ends_with("bawkshot.exe"))
        .ok_or("the download has no bawkshot.exe in it")?;
    let mut entry = archive.by_name(&name).map_err(|err| err.to_string())?;
    let mut target = File::create(out).map_err(|err| {
        format!("cannot write next to bawkshot.exe ({err}). download the new version by hand.")
    })?;
    std::io::copy(&mut entry, &mut target).map_err(|err| err.to_string())?;
    Ok(())
}

/// Windows lets a running exe be renamed but not overwritten, so the old one steps aside first.
fn swap(exe: &Path, fresh: &Path) -> Result<(), String> {
    let old = sibling(exe, "old");
    let _ = fs::remove_file(&old);
    fs::rename(exe, &old).map_err(|err| format!("cannot replace bawkshot.exe: {err}"))?;
    if let Err(err) = fs::rename(fresh, exe) {
        let _ = fs::rename(&old, exe);
        return Err(format!("cannot replace bawkshot.exe: {err}"));
    }
    Ok(())
}

/// Starts the freshly installed version; the caller quits this one.
pub fn relaunch(exe: &Path) -> Result<(), String> {
    Command::new(exe)
        .arg(AFTER_UPDATE)
        .spawn()
        .map(|_| ())
        .map_err(|err| format!("could not start the new version: {err}"))
}

/// Removes the exe an earlier update set aside, waiting for that copy to finish quitting.
pub fn clean_up() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let old = sibling(&exe, "old");
    if !old.exists() {
        return;
    }
    std::thread::spawn(move || {
        for _ in 0..30 {
            if fs::remove_file(&old).is_ok() || !old.exists() {
                return;
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    });
}

/// An update swaps the exe without the installer, so the version Windows lists for an installed copy is corrected here.
pub fn sync_installed_version() {
    if let Ok(exe) = std::env::current_exe() {
        sync_version(UNINSTALL_KEY, &exe, VERSION);
    }
}

fn sync_version(key: &str, exe: &Path, version: &str) {
    let Ok(entry) = CURRENT_USER.options().read().write().open(key) else {
        return;
    };
    let location = entry.get_string("InstallLocation").unwrap_or_default();
    let ours = exe.parent().is_some_and(|dir| {
        dir.to_string_lossy()
            .eq_ignore_ascii_case(location.trim_end_matches('\\'))
    });
    let shown = entry.get_string("DisplayVersion").unwrap_or_default();
    let Some((major, minor, _)) = parse_version(version).filter(|_| ours && shown != version)
    else {
        return;
    };
    let _ = entry.set_string("DisplayVersion", version);
    for (name, value) in [
        ("MajorVersion", major),
        ("VersionMajor", major),
        ("MinorVersion", minor),
        ("VersionMinor", minor),
    ] {
        let _ = entry.set_u32(name, value as u32);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::net::TcpListener;

    fn asset(name: &str) -> ApiAsset {
        ApiAsset {
            name: name.into(),
            browser_download_url: format!("https://example.com/{name}"),
            size: 10,
        }
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("bawkshot-{name}-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn zip_with(name: &str, bytes: &[u8]) -> Vec<u8> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file(name, zip::write::SimpleFileOptions::default())
            .unwrap();
        writer.write_all(bytes).unwrap();
        writer.finish().unwrap().into_inner()
    }

    /// A stand-in for GitHub: answers each path with the bytes given, and 404 for the rest.
    fn serve(listener: TcpListener, files: Vec<(String, Vec<u8>)>) {
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut request = Vec::new();
                let mut byte = [0u8; 1];
                while !request.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                    request.push(byte[0]);
                }
                let request = String::from_utf8_lossy(&request).into_owned();
                let path = request.split_whitespace().nth(1).unwrap_or_default();
                let found = files.iter().find(|(name, _)| name == path);
                let (status, body) = match found {
                    Some((_, body)) => ("200 OK", body.as_slice()),
                    None => ("404 Not Found", &b""[..]),
                };
                let head = format!(
                    "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(body);
            }
        });
    }

    /// Publishes release 99.0.0 on a local server; `sum` overrides the checksum it advertises.
    fn publish(exe_bytes: &[u8], sum: Option<&str>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let zip_name = format!("bawkshot-v99.0.0{ASSET_SUFFIX}");
        let zip = zip_with("bawkshot-v99.0.0-windows-x64/bawkshot.exe", exe_bytes);
        let sum = sum.map_or_else(|| hex(&Sha256::digest(&zip)), str::to_string);
        let latest = format!(
            r#"{{"tag_name":"v99.0.0","html_url":"{base}/notes","assets":[
                {{"name":"{zip_name}","browser_download_url":"{base}/zip","size":{}}},
                {{"name":"{zip_name}.sha256","browser_download_url":"{base}/sum","size":90}}]}}"#,
            zip.len()
        );
        let files = vec![
            ("/latest".to_string(), latest.into_bytes()),
            ("/zip".to_string(), zip),
            (
                "/sum".to_string(),
                format!("{sum}  {zip_name}\n").into_bytes(),
            ),
        ];
        serve(listener, files);
        base
    }

    #[test]
    fn compares_versions() {
        assert_eq!(parse_version("v1.2.3"), Some((1, 2, 3)));
        assert_eq!(parse_version("0.10.0-beta.1"), Some((0, 10, 0)));
        assert_eq!(parse_version("nonsense"), None);
        assert!(is_newer("v0.2.0", "0.1.9"));
        assert!(is_newer("v0.10.0", "0.9.0"));
        assert!(!is_newer("v0.1.0", "0.1.0"));
        assert!(!is_newer("garbage", "0.1.0"));
    }

    #[test]
    fn picks_the_zip_and_its_checksum_not_the_installer() {
        let ours = format!("bawkshot-v0.2.0{ASSET_SUFFIX}");
        let release = ApiRelease {
            tag_name: "v0.2.0".into(),
            html_url: "https://example.com/v0.2.0".into(),
            assets: vec![
                asset("bawkshot-v0.2.0-windows-x64-setup.exe"),
                asset("bawkshot-v0.2.0-windows-x64-setup.exe.sha256"),
                asset("bawkshot-v0.2.0-windows-x64.zip.sha256"),
                asset("bawkshot-v0.2.0-windows-x64.zip"),
            ],
        };
        let picked = pick(release).unwrap();
        assert_eq!(picked.version, "0.2.0");
        assert!(picked.asset_url.ends_with(&ours));
        assert!(picked.sum_url.ends_with(&format!("{ours}.sha256")));
        let bare = ApiRelease {
            tag_name: "v0.2.0".into(),
            html_url: String::new(),
            assets: vec![asset(&ours)],
        };
        assert!(pick(bare).is_err());
    }

    #[test]
    fn swaps_files_and_names_siblings() {
        let dir = scratch("swap");
        let exe = dir.join("bawkshot.exe");
        let fresh = sibling(&exe, "new");
        assert_eq!(fresh.file_name().unwrap(), "bawkshot.new.exe");
        fs::write(&exe, b"old").unwrap();
        fs::write(&fresh, b"new").unwrap();
        swap(&exe, &fresh).unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"new");
        assert_eq!(fs::read(sibling(&exe, "old")).unwrap(), b"old");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn an_update_is_found_downloaded_checked_and_swapped_in() {
        let base = publish(b"the new exe", None);
        let release = check_at(&format!("{base}/latest")).unwrap().unwrap();
        assert_eq!(release.version, "99.0.0");
        assert_eq!(release.page, format!("{base}/notes"));

        let dir = scratch("install");
        let exe = dir.join("bawkshot.exe");
        fs::write(&exe, b"the old exe").unwrap();
        let seen = std::cell::Cell::new(0);
        install_to(&release, &exe, |bytes| seen.set(bytes)).unwrap();
        assert_eq!(fs::read(&exe).unwrap(), b"the new exe");
        assert_eq!(fs::read(sibling(&exe, "old")).unwrap(), b"the old exe");
        assert_eq!(seen.get(), release.size);
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 2);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_download_that_fails_its_checksum_is_not_installed() {
        let base = publish(b"tampered", Some(&"0".repeat(64)));
        let release = check_at(&format!("{base}/latest")).unwrap().unwrap();
        let dir = scratch("checksum");
        let exe = dir.join("bawkshot.exe");
        fs::write(&exe, b"the old exe").unwrap();
        let error = install_to(&release, &exe, |_| {}).unwrap_err();
        assert!(error.contains("checksum"), "{error}");
        assert_eq!(fs::read(&exe).unwrap(), b"the old exe");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_missing_release_and_an_old_one_are_told_apart() {
        let base = publish(b"exe", None);
        let error = check_at(&format!("{base}/nothing-here")).unwrap_err();
        assert!(error.contains("no published release"), "{error}");

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let api = format!("http://{}/latest", listener.local_addr().unwrap());
        let same = format!(r#"{{"tag_name":"v{VERSION}","html_url":"","assets":[]}}"#);
        serve(listener, vec![("/latest".to_string(), same.into_bytes())]);
        assert_eq!(check_at(&api), Ok(None));
    }

    #[test]
    fn the_listed_version_is_corrected_only_for_this_copy() {
        let key = format!(r"Software\bawkshot-test-updater-{}", std::process::id());
        let entry = CURRENT_USER.create(&key).unwrap();
        entry
            .set_string("InstallLocation", r"C:\Apps\bawkshot\")
            .unwrap();
        entry.set_string("DisplayVersion", "0.1.0").unwrap();
        entry.set_u32("VersionMinor", 1).unwrap();

        sync_version(&key, Path::new(r"D:\elsewhere\bawkshot.exe"), "0.3.0");
        assert_eq!(entry.get_string("DisplayVersion").unwrap(), "0.1.0");
        sync_version(&key, Path::new(r"c:\apps\bawkshot\bawkshot.exe"), "0.3.0");
        assert_eq!(entry.get_string("DisplayVersion").unwrap(), "0.3.0");
        assert_eq!(entry.get_u32("VersionMinor").unwrap(), 3);
        CURRENT_USER.remove_tree(&key).unwrap();
    }

    #[test]
    fn the_uninstall_key_and_the_names_match_the_installer() {
        let script = include_str!("../packaging/windows/bawkshot.iss");
        let id = UNINSTALL_KEY
            .rsplit('\\')
            .next()
            .unwrap()
            .trim_end_matches("_is1");
        assert!(script.contains(&format!("AppId={{{id}")));
        assert!(script.contains(r"{app}\bawkshot.old.exe"));
        let package = include_str!("../packaging/windows/package.ps1");
        assert_eq!(ASSET_SUFFIX, "-windows-x64.zip");
        assert!(package.contains("\"bawkshot-v$version-windows-x64\""));
        assert!(package.contains("\"dist/$name.zip\""));
    }
}
