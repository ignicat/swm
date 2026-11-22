//! Exercise the real lifecycle helpers in isolated processes with a fake manager.
#[allow(dead_code)]
mod environment {
    include!("../src/session/startup/environment.rs");
    #[cfg(test)]
    mod tests {
        include!("support/startup_environment_unit_tests.rs");
    }
}

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

struct Scratch(PathBuf);
impl Scratch {
    fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "swm-session-lifecycle-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir(&path).unwrap();
        Self(path)
    }
    fn executable(&self, name: &str, script: &str) -> PathBuf {
        let path = self.0.join(name);
        fs::write(&path, script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        path
    }
}
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
#[ignore = "subprocess helper for the lifecycle regression tests"]
fn lifecycle_child() {
    assert_eq!(std::env::var("SWM_LIFECYCLE_CHILD").unwrap(), "1");
    let _native = environment::NativeSession;
    if std::env::var_os("READY").is_some() {
        environment::notify_ready();
    }
    if std::env::var_os("EARLY_SHUTDOWN").is_some() {
        environment::shutdown_session();
        environment::shutdown_session();
    }
}

fn lifecycle_calls(ready: bool, managed: bool, fail_start: bool) -> String {
    let scratch = Scratch::new();
    scratch.executable("systemctl", "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$CAPTURE\"\n# Mirror systemd's RefuseManualStart on the generic graphical target.\nif [ \"$2\" = start ] && [ \"$4\" = graphical-session.target ]; then exit 4; fi\nif [ \"${FAIL_START:-}\" = 1 ] && [ \"$2\" = start ]; then exit 1; fi\n");
    let capture = scratch.0.join("calls");
    let mut process = Command::new(std::env::current_exe().unwrap());
    process
        .args(["--ignored", "--exact", "lifecycle_child"])
        .env_clear()
        .env("PATH", &scratch.0)
        .env("CAPTURE", &capture)
        .env("SWM_LIFECYCLE_CHILD", "1");
    if ready {
        process.env("READY", "1");
        process.env("EARLY_SHUTDOWN", "1");
    }
    if managed {
        process.env("SWM_SESSION_LAUNCHER_ACTIVE", "1");
    }
    if fail_start {
        process.env("FAIL_START", "1");
    }
    let output = process.output().unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stdout)
    );
    fs::read_to_string(capture).unwrap_or_default()
}

#[test]
fn direct_ready_session_starts_target_then_shuts_down_and_clears_display_environment() {
    if !cfg!(feature = "systemd") || !Path::new("/run/systemd/system").is_dir() {
        return;
    }
    let calls = lifecycle_calls(true, false, false);
    let lines: Vec<_> = calls.lines().collect();
    assert_eq!(lines.len(), 3, "{calls}");
    assert_eq!(
        lines[0],
        "--user start --no-block swm-direct-session.target"
    );
    assert_eq!(
        lines[1],
        "--user start --job-mode=replace-irreversibly swm-shutdown.target"
    );
    assert!(lines[2].starts_with("--user unset-environment WAYLAND_DISPLAY DISPLAY "));
    assert!(!lines[2].split_whitespace().any(|word| word == "PATH"));
}

#[test]
fn managed_or_unready_sessions_do_not_take_ownership_of_host_targets() {
    assert!(lifecycle_calls(true, true, false).is_empty());
    assert!(lifecycle_calls(false, false, false).is_empty());
}

#[test]
fn failed_target_start_does_not_stop_an_existing_graphical_session() {
    let calls = lifecycle_calls(true, false, true);
    assert!(!calls.contains("shutdown"));
    assert!(!calls.contains("stop"));
    assert!(!calls.contains("unset-environment"));
}

#[test]
fn managed_launcher_exec_preserves_main_pid_selected_binary_and_arguments() {
    let scratch = Scratch::new();
    scratch.executable("ps", "#!/bin/sh\nprintf '%s\\n' 'systemd --user'\n");
    let binary = scratch.executable(
        "selected-swm",
        "#!/bin/sh\nprintf '%s\\n' \"$$\" \"$@\" > \"$CAPTURE\"\n",
    );
    let capture = scratch.0.join("exec");
    let launcher =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("packaging/wayland-sessions/swm-session");
    let mut child = Command::new("/bin/sh")
        .args([
            "-c",
            "SYSTEMD_EXEC_PID=$$; export SYSTEMD_EXEC_PID; exec /bin/sh \"$1\" --config \"$2\"",
            "sh",
        ])
        .arg(launcher)
        .arg("config with spaces.rune")
        .env_clear()
        .env("PATH", &scratch.0)
        .env("MANAGERPID", "1234")
        .env("SWM_BIN", binary)
        .env("CAPTURE", &capture)
        .spawn()
        .unwrap();
    let pid = child.id();
    assert!(child.wait().unwrap().success());
    assert_eq!(
        fs::read_to_string(capture).unwrap(),
        format!("{pid}\n--session\n--config\nconfig with spaces.rune\n")
    );
}

#[test]
fn direct_launcher_selects_sibling_install_and_preserves_explicit_arguments() {
    let scratch = Scratch::new();
    scratch.executable("swm", "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$CAPTURE\"\n");
    let launcher = scratch.0.join("swm-session");
    fs::copy(
        Path::new(env!("CARGO_MANIFEST_DIR")).join("packaging/wayland-sessions/swm-session"),
        &launcher,
    )
    .unwrap();
    let capture = scratch.0.join("arguments");
    let status = Command::new("/bin/sh")
        .arg(launcher)
        .args(["--config", "config with spaces.rune"])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("SWM_NO_INIT_INTEGRATION", "1")
        .env("SWM_SESSION_LOGIN_READY", "1")
        .env("CAPTURE", &capture)
        .status()
        .unwrap();
    assert!(status.success());
    assert_eq!(
        fs::read_to_string(capture).unwrap(),
        "--session\n--config\nconfig with spaces.rune\n"
    );
}

#[test]
fn dinit_wrapper_preserves_explicit_arguments_instead_of_dropping_them() {
    let scratch = Scratch::new();
    let binary = scratch.executable("swm", "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$CAPTURE\"\n");
    // Simulate a distro install under /usr/bin/swm while fake dinitctl/pgrep/units exist.
    let dinit_dir = scratch.0.join("dinit.d");
    fs::create_dir(&dinit_dir).unwrap();
    fs::write(dinit_dir.join("swm"), "type = process\n").unwrap();
    fs::write(dinit_dir.join("swm.target"), "type = internal\n").unwrap();
    let dinit_called = scratch.0.join("dinit-called");
    scratch.executable("dinitctl", "#!/bin/sh\ntouch \"$DINIT_CALLED\"\n");
    scratch.executable("dinit-monitor", "#!/bin/sh\nexit 0\n");
    scratch.executable("pgrep", "#!/bin/sh\nexit 0\n");
    let capture = scratch.0.join("arguments");
    let launcher =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("packaging/wayland-sessions/swm-session");
    let status = Command::new("/bin/sh")
        .arg(launcher)
        .args(["--config", "explicit dinit.rune"])
        .env_clear()
        .env("PATH", &scratch.0)
        .env("HOME", &scratch.0)
        .env("XDG_CONFIG_HOME", &scratch.0)
        .env("SWM_BIN", binary)
        .env("SWM_SESSION_LOGIN_READY", "1")
        .env("DINIT_CALLED", &dinit_called)
        .env("CAPTURE", &capture)
        .status()
        .unwrap();
    assert!(status.success());
    assert!(!dinit_called.exists());
    assert_eq!(
        fs::read_to_string(capture).unwrap(),
        "--session\n--config\nexplicit dinit.rune\n"
    );
}
