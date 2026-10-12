use windows_registry::CURRENT_USER;

const RUN_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const VALUE: &str = "bawkshot";

/// The command Windows runs at sign-in: this exe, quoted in case its path has spaces.
fn launch_command() -> Option<String> {
    let exe = std::env::current_exe().ok()?;
    Some(format!("\"{}\"", exe.display()))
}

fn read(key: &str) -> Option<String> {
    CURRENT_USER.open(key).ok()?.get_string(VALUE).ok()
}

fn write(key: &str, on: bool) -> Result<(), String> {
    let run = CURRENT_USER
        .create(key)
        .map_err(|err| format!("could not open the startup list: {err}"))?;
    if on {
        let command = launch_command().ok_or("could not find this program's own path")?;
        run.set_string(VALUE, command)
            .map_err(|err| format!("could not add bawkshot to the startup list: {err}"))
    } else if read(key).is_some() {
        run.remove_value(VALUE)
            .map_err(|err| format!("could not remove bawkshot from the startup list: {err}"))
    } else {
        Ok(())
    }
}

/// True only when the entry starts this copy; one left by a build that has since moved counts as off.
fn points_here(key: &str) -> bool {
    let (Some(entry), Some(command)) = (read(key), launch_command()) else {
        return false;
    };
    entry.eq_ignore_ascii_case(&command)
}

pub fn enabled() -> bool {
    points_here(RUN_KEY)
}

pub fn set(on: bool) -> Result<(), String> {
    write(RUN_KEY, on)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_entry_is_written_read_and_removed() {
        let key = format!(r"Software\bawkshot-test-{}", std::process::id());
        assert!(!points_here(&key));
        write(&key, true).unwrap();
        assert!(points_here(&key));
        assert!(read(&key).unwrap().starts_with('"'));
        write(&key, false).unwrap();
        assert!(!points_here(&key));
        write(&key, false).unwrap();
        CURRENT_USER.remove_tree(&key).unwrap();
    }
}
