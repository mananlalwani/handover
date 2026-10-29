use super::*;

use handover_core::{DeviceId, MediaSessionId};
use std::collections::BTreeSet;
use zbus::zvariant::Value;

fn fixture_state(playback: PlaybackState, controls: &[MediaControl]) -> State {
    State {
        session: MediaSession {
            id: MediaSessionId::new(DeviceId::new("native:phone-a"), "com.example.player"),
            application: "Example player".into(),
            title: Some("A track".into()),
            artist: Some("An artist".into()),
            album: Some("An album".into()),
            playback,
            position_ms: Some(12_345),
            duration_ms: Some(234_567),
            volume_percent: None,
            controls: controls.iter().copied().collect::<BTreeSet<_>>(),
        },
        device_name: "Phone A".into(),
        active: true,
        revision: 7,
    }
}

fn fixture_player(state: State) -> Player {
    fixture_player_at(state, PathBuf::from("/this/path/must/not/be/used"))
}

fn fixture_player_at(state: State, socket: PathBuf) -> Player {
    Player {
        state: Arc::new(RwLock::new(state)),
        socket,
        commands: Arc::new(Semaphore::new(1)),
    }
}

fn assert_unsupported<T>(result: fdo::Result<T>) {
    assert!(matches!(result, Err(fdo::Error::NotSupported(_))));
}

#[test]
fn known_playback_states_map_and_unknown_is_not_guessed() {
    assert_eq!(
        fixture_state(PlaybackState::Playing, &[]).status().unwrap(),
        "Playing"
    );
    assert_eq!(
        fixture_state(PlaybackState::Paused, &[]).status().unwrap(),
        "Paused"
    );
    assert_eq!(
        fixture_state(PlaybackState::Stopped, &[]).status().unwrap(),
        "Stopped"
    );
    assert!(matches!(
        fixture_state(PlaybackState::Unknown, &[]).status(),
        Err(fdo::Error::NotSupported(_))
    ));
}

#[test]
fn metadata_uses_dbus_track_path_artist_array_and_microsecond_length() {
    let metadata = fixture_state(PlaybackState::Playing, &[]).metadata();

    assert_eq!(metadata["mpris:trackid"].value_signature().to_string(), "o");
    match &metadata["mpris:trackid"] {
        Value::ObjectPath(path) => assert_eq!(path.as_str(), "/org/handover/Media/observed7"),
        other => panic!("track ID must be a D-Bus object path, got {other:?}"),
    }
    assert_eq!(
        metadata["xesam:artist"]
            .clone()
            .downcast::<Vec<String>>()
            .unwrap(),
        vec!["An artist"]
    );
    assert_eq!(
        metadata["mpris:length"].clone().downcast::<i64>().unwrap(),
        234_567_000
    );
    assert_eq!(
        metadata["xesam:title"]
            .clone()
            .downcast::<String>()
            .unwrap(),
        "A track"
    );
    assert_eq!(
        metadata["xesam:album"]
            .clone()
            .downcast::<String>()
            .unwrap(),
        "An album"
    );
}

#[test]
fn missing_track_metadata_and_duration_are_omitted() {
    let mut state = fixture_state(PlaybackState::Playing, &[]);
    state.session.title = None;
    state.session.artist = None;
    state.session.album = None;
    state.session.duration_ms = None;

    let metadata = state.metadata();

    assert!(metadata.contains_key("mpris:trackid"));
    assert!(!metadata.contains_key("xesam:title"));
    assert!(!metadata.contains_key("xesam:artist"));
    assert!(!metadata.contains_key("xesam:album"));
    assert!(!metadata.contains_key("mpris:length"));
}

#[test]
fn time_conversion_checks_overflow_and_position_is_the_latest_sample() {
    assert_eq!(microseconds(12_345), Some(12_345_000));
    assert_eq!(microseconds(u64::MAX), None);
    assert_eq!(
        microseconds((i64::MAX as u64) / 1000),
        Some(i64::MAX / 1000 * 1000)
    );
    assert_eq!(microseconds((i64::MAX as u64) / 1000 + 1), None);

    let player = fixture_player(fixture_state(PlaybackState::Playing, &[]));
    assert_eq!(player.position().unwrap(), 12_345_000);
}

#[test]
fn absent_position_and_phone_volume_are_errors_not_fabricated_defaults() {
    let mut state = fixture_state(PlaybackState::Playing, &[]);
    state.session.position_ms = None;
    let player = fixture_player(state);
    assert_unsupported(player.position());
    assert_unsupported(player.volume());

    let mut state = fixture_state(PlaybackState::Playing, &[]);
    state.session.volume_percent = Some(73);
    assert_eq!(fixture_player(state).volume().unwrap(), 0.73);

    let mut state = fixture_state(PlaybackState::Playing, &[]);
    state.session.volume_percent = Some(101);
    assert_unsupported(fixture_player(state).volume());
}

#[test]
fn player_identity_includes_device_name() {
    let first = fixture_state(PlaybackState::Playing, &[]);
    let mut second = fixture_state(PlaybackState::Playing, &[]);
    second.device_name = "Phone B".into();
    assert_ne!(first.identity(), second.identity());
    assert!(first.identity().contains("Phone A"));
    assert!(second.identity().contains("Phone B"));
}

#[test]
fn direct_capability_flags_do_not_infer_play_or_pause_from_toggle() {
    let player = fixture_player(fixture_state(
        PlaybackState::Playing,
        &[
            MediaControl::PlayPause,
            MediaControl::Previous,
            MediaControl::SetPosition,
        ],
    ));

    assert!(!player.can_play());
    assert!(!player.can_pause());
    assert!(!player.can_go_next());
    assert!(player.can_go_previous());
    assert!(!player.can_seek());
}

#[test]
fn inactive_player_advertises_no_action_capabilities() {
    let mut state = fixture_state(
        PlaybackState::Playing,
        &[
            MediaControl::Play,
            MediaControl::Pause,
            MediaControl::PlayPause,
            MediaControl::Next,
            MediaControl::Previous,
            MediaControl::SetPosition,
        ],
    );
    state.active = false;
    let player = fixture_player(state);

    assert!(!player.can_play());
    assert!(!player.can_pause());
    assert!(!player.can_go_next());
    assert!(!player.can_go_previous());
    assert!(!player.can_seek());
}

#[tokio::test]
async fn unsupported_controls_and_position_methods_fail_without_ipc() {
    let player = fixture_player(fixture_state(
        PlaybackState::Playing,
        &[MediaControl::PlayPause],
    ));
    assert_unsupported(player.play().await);
    assert_unsupported(player.play_pause().await);
    assert_unsupported(player.stop());
    assert_unsupported(player.seek(1));
    assert_unsupported(player.set_position(
        OwnedObjectPath::try_from("/org/handover/Media/observed7").unwrap(),
        10,
    ));
    assert_unsupported(player.open_uri("https://example.invalid"));
    assert!(player.set_rate(2.0).is_err());
    assert!(player.set_volume(0.5).is_err());
}

#[tokio::test]
async fn pending_media_command_is_not_retried_or_falsely_applied() {
    let directory = tempfile::tempdir().unwrap();
    let socket = directory.path().join("daemon.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let player = Arc::new(fixture_player_at(
        fixture_state(
            PlaybackState::Playing,
            &[MediaControl::Pause, MediaControl::Next],
        ),
        socket,
    ));

    let pending_player = Arc::clone(&player);
    let pause = tokio::spawn(async move { pending_player.pause().await });
    let (mut command_socket, _) =
        tokio::time::timeout(std::time::Duration::from_secs(1), listener.accept())
            .await
            .expect("pause command should connect")
            .unwrap();
    let request: handover_ipc::Request =
        handover_ipc::read_json_line(&mut tokio::io::BufReader::new(&mut command_socket))
            .await
            .unwrap()
            .unwrap();
    assert!(matches!(
        request.method,
        handover_ipc::Method::MediaControl {
            command: MediaCommand::Pause { .. }
        }
    ));

    assert!(matches!(
        player.next().await,
        Err(fdo::Error::LimitsExceeded(_))
    ));
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(150), listener.accept(),)
            .await
            .is_err(),
        "second control must not open another IPC connection"
    );

    let result = tokio::time::timeout(std::time::Duration::from_secs(4), pause)
        .await
        .expect("media command uses the production timeout")
        .unwrap();
    assert!(matches!(result, Err(fdo::Error::Timeout(_))));
    assert_eq!(
        player.state.read().unwrap().session.playback,
        PlaybackState::Playing,
        "accepted command timeout must not mutate observed playback state"
    );
    assert!(
        tokio::time::timeout(std::time::Duration::from_millis(150), listener.accept(),)
            .await
            .is_err(),
        "ambiguous command must not be retried"
    );
}
