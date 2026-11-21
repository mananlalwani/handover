use std::fs;
use std::os::unix::{fs::PermissionsExt, net::UnixListener};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

#[test]
fn duplicate_daemon_does_not_recover_the_running_daemons_audio_routes() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = directory.path().join("runtime");
    fs::create_dir_all(runtime.join("handover")).unwrap();
    let _running_daemon = UnixListener::bind(runtime.join("handover/handoverd.sock")).unwrap();
    let native = directory.path().join("state/handover/native");
    fs::create_dir_all(&native).unwrap();
    let journal = native.join("audio-routing.json");
    let contents = br#"{"sink":"handover_phone_fixture","module":42,"default_sink":"fixture-speakers","muted":{},"inputs":{},"mirror":null}"#;
    fs::write(&journal, contents).unwrap();
    let binaries = directory.path().join("bin");
    fs::create_dir(&binaries).unwrap();
    let pactl = binaries.join("pactl");
    fs::write(
        &pactl,
        "#!/bin/sh\n: > \"$HANDOVER_TEST_PACTL_MARKER\"\nexit 1\n",
    )
    .unwrap();
    fs::set_permissions(&pactl, fs::Permissions::from_mode(0o755)).unwrap();
    let marker = directory.path().join("audio-was-touched");
    let mut child = Command::new(env!("CARGO_BIN_EXE_handoverd"))
        .env("HOME", directory.path())
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("XDG_STATE_HOME", directory.path().join("state"))
        .env("XDG_CONFIG_HOME", directory.path().join("config"))
        .env("XDG_DATA_HOME", directory.path().join("data"))
        .env("XDG_CACHE_HOME", directory.path().join("cache"))
        .env("PATH", &binaries)
        .env("HANDOVER_TEST_PACTL_MARKER", &marker)
        .env_remove("DBUS_SESSION_BUS_ADDRESS")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while child.try_wait().unwrap().is_none() {
        if Instant::now() >= deadline {
            child.kill().unwrap();
            child.wait().unwrap();
            panic!("duplicate daemon did not exit promptly");
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        !marker.exists(),
        "duplicate startup must not invoke audio recovery against active routes"
    );
    assert_eq!(fs::read(journal).unwrap(), contents);
}

#[test]
fn unsupported_arguments_exit_before_runtime_or_state_initialization() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = directory.path().join("runtime");
    let state = directory.path().join("state");
    let output = Command::new(env!("CARGO_BIN_EXE_handoverd"))
        .arg("--unknown-option")
        .env("HOME", directory.path())
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("XDG_STATE_HOME", &state)
        .output()
        .unwrap();
    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("accepts only --version"));
    assert!(!runtime.exists());
    assert!(!state.exists());
}

#[test]
fn version_query_is_read_only() {
    let directory = tempfile::tempdir().unwrap();
    let runtime = directory.path().join("runtime");
    let state = directory.path().join("state");
    for flag in ["--version", "-V"] {
        let output = Command::new(env!("CARGO_BIN_EXE_handoverd"))
            .arg(flag)
            .env("HOME", directory.path())
            .env("XDG_RUNTIME_DIR", &runtime)
            .env("XDG_STATE_HOME", &state)
            .output()
            .unwrap();
        assert!(output.status.success());
        assert_eq!(
            String::from_utf8(output.stdout).unwrap().trim(),
            concat!("handoverd ", env!("CARGO_PKG_VERSION"))
        );
        assert!(!runtime.exists());
        assert!(!state.exists());
    }
}
