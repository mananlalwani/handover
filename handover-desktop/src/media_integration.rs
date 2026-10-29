use crate::media_player::{PATH, PLAYER, Player, ROOT, Root, State};
use crate::{CALL_TIMEOUT, integration::private_bus, run_media};
use futures_util::StreamExt;
use handover_core::{
    Capability, Device, DeviceId, MediaCommand, MediaControl, MediaSession, MediaSessionId,
    PlaybackState,
};
use handover_ipc::{
    Method, Request, ServerMessage, ServerPayload, read_json_line, write_json_line,
};
use std::collections::{BTreeSet, HashMap};
use std::sync::{Arc, RwLock};
use std::time::Duration;
use tokio::io::BufReader;
use tokio::net::{UnixListener, UnixStream};
use tokio::process::Command;
use tokio::sync::{Semaphore, watch};
use tokio::time::timeout;
use zbus::{Connection, Proxy, proxy::SignalStream, zvariant::OwnedValue};

fn device() -> Device {
    Device {
        id: DeviceId::new("fixture"),
        name: "Fixture phone".into(),
        connected: true,
        paired: true,
        battery: None,
        connectivity: None,
        capabilities: BTreeSet::from([Capability::Media]),
    }
}

fn session() -> MediaSession {
    MediaSession {
        id: MediaSessionId::new(DeviceId::new("fixture"), "player"),
        application: "Fixture app".into(),
        title: Some("Track".into()),
        artist: Some("Artist".into()),
        album: None,
        playback: PlaybackState::Playing,
        position_ms: Some(1234),
        duration_ms: Some(60_000),
        volume_percent: None,
        controls: BTreeSet::from([
            MediaControl::Play,
            MediaControl::Pause,
            MediaControl::PlayPause,
            MediaControl::Next,
        ]),
    }
}

async fn send(socket: &mut UnixStream, payload: ServerPayload) {
    write_json_line(socket, &ServerMessage::new(payload))
        .await
        .unwrap();
}

async fn subscribe(listener: &UnixListener, media: Vec<MediaSession>) -> UnixStream {
    let (mut stream, _) = timeout(Duration::from_secs(6), listener.accept())
        .await
        .unwrap()
        .unwrap();
    let request: Request = read_json_line(&mut BufReader::new(&mut stream))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        request.method,
        Method::Subscribe {
            shares: false,
            media: true,
            messages: false
        }
    );
    send(
        &mut stream,
        ServerPayload::Subscribed {
            devices: vec![device()],
            notifications: vec![],
            media_sessions: media,
            calls: vec![],
            messaging_accounts: vec![],
            conversations: vec![],
            typing_states: vec![],
            read_states: vec![],
        },
    )
    .await;
    stream
}

async fn owner(stream: &mut SignalStream<'_>, expected: &str, present: bool) {
    timeout(Duration::from_secs(6), async {
        loop {
            let signal = stream.next().await.unwrap();
            let (name, _old, new): (String, String, String) = signal.body().deserialize().unwrap();
            if name == expected && new.is_empty() != present {
                return;
            }
        }
    })
    .await
    .unwrap();
}

async fn command(listener: &UnixListener, expected: MediaCommand) {
    let (mut stream, _) = timeout(CALL_TIMEOUT, listener.accept())
        .await
        .unwrap()
        .unwrap();
    let request: Request = read_json_line(&mut BufReader::new(&mut stream))
        .await
        .unwrap()
        .unwrap();
    assert_eq!(
        request.method,
        Method::MediaControl {
            command: expected.clone()
        }
    );
    send(
        &mut stream,
        ServerPayload::MediaAccepted {
            id: expected.id().clone(),
        },
    )
    .await;
}

async fn client(address: &str) -> Connection {
    zbus::connection::Builder::address(address)
        .unwrap()
        .build()
        .await
        .unwrap()
}

#[tokio::test]
async fn mpris_discovery_commands_updates_and_recovery_use_reported_state() {
    let (_bus, address) = private_bus().await;
    let connection = client(&address).await;
    let bus = Proxy::new(
        &connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )
    .await
    .unwrap();
    let mut owners = bus.receive_signal("NameOwnerChanged").await.unwrap();
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("daemon.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let (shutdown, receiver) = watch::channel(false);
    // The notification service is deliberately absent. Its independent runner
    // must neither hold up the media subscription nor prevent clean shutdown.
    let notification_worker = tokio::spawn(crate::run(
        path.clone(),
        connection.clone(),
        receiver.clone(),
    ));
    let worker = tokio::spawn(run_media(path, address.clone(), receiver));
    let initial = session();
    let mut daemon = subscribe(&listener, vec![initial.clone()]).await;
    let name = "org.mpris.MediaPlayer2.handover.player1";
    owner(&mut owners, name, true).await;
    let proxy = Proxy::new(&connection, name, PATH, PLAYER).await.unwrap();
    let root = Proxy::new(&connection, name, PATH, ROOT).await.unwrap();
    assert_eq!(
        root.get_property::<String>("Identity").await.unwrap(),
        "Fixture app on Fixture phone"
    );
    assert_eq!(
        proxy
            .get_property::<String>("PlaybackStatus")
            .await
            .unwrap(),
        "Playing"
    );
    assert_eq!(
        proxy.get_property::<i64>("Position").await.unwrap(),
        1_234_000
    );
    let metadata: HashMap<String, OwnedValue> = proxy.get_property("Metadata").await.unwrap();
    assert_eq!(
        i64::try_from(&metadata["mpris:length"]).unwrap(),
        60_000_000
    );
    assert_eq!(<&str>::try_from(&metadata["xesam:title"]).unwrap(), "Track");
    assert!(!proxy.get_property::<bool>("CanSeek").await.unwrap());
    assert!(!proxy.get_property::<bool>("CanGoPrevious").await.unwrap());
    assert!(proxy.get_property::<f64>("Volume").await.is_err());
    let props = Proxy::new(&connection, name, PATH, "org.freedesktop.DBus.Properties")
        .await
        .unwrap();
    let mut changed = props.receive_signal("PropertiesChanged").await.unwrap();

    // Command acceptance must not manufacture a paused state.
    let (result, ()) = tokio::join!(
        proxy.call::<_, _, ()>("Pause", &()),
        command(
            &listener,
            MediaCommand::Pause {
                id: initial.id.clone()
            }
        )
    );
    result.unwrap();
    assert_eq!(
        proxy
            .get_property::<String>("PlaybackStatus")
            .await
            .unwrap(),
        "Playing"
    );
    let mut paused = initial.clone();
    paused.playback = PlaybackState::Paused;
    paused.title = Some("Next track".into());
    send(
        &mut daemon,
        ServerPayload::MediaUpdated {
            media_session: paused.clone(),
        },
    )
    .await;
    let update = timeout(CALL_TIMEOUT, changed.next())
        .await
        .unwrap()
        .unwrap();
    let (interface, values, invalidated): (String, HashMap<String, OwnedValue>, Vec<String>) =
        update.body().deserialize().unwrap();
    assert_eq!(interface, PLAYER);
    assert!(!values.contains_key("Position"));
    assert!(invalidated.is_empty());
    assert_eq!(
        <&str>::try_from(&values["PlaybackStatus"]).unwrap(),
        "Paused"
    );
    assert_eq!(
        proxy
            .get_property::<String>("PlaybackStatus")
            .await
            .unwrap(),
        "Paused"
    );
    let updated: HashMap<String, OwnedValue> = proxy.get_property("Metadata").await.unwrap();
    assert_ne!(updated["mpris:trackid"], metadata["mpris:trackid"]);
    assert!(proxy.call::<_, _, ()>("Previous", &()).await.is_err());
    assert!(
        proxy
            .call::<_, _, ()>("Seek", &(1_000_000i64,))
            .await
            .is_err()
    );
    assert!(proxy.set_property("Volume", 0.5f64).await.is_err());
    assert!(
        timeout(Duration::from_millis(80), listener.accept())
            .await
            .is_err(),
        "unsupported commands must not reach IPC"
    );

    // Position samples must remain queryable without false metadata or status
    // changes. A subsequent capability update provides an event barrier.
    paused.position_ms = Some(5000);
    send(
        &mut daemon,
        ServerPayload::MediaUpdated {
            media_session: paused.clone(),
        },
    )
    .await;
    paused.controls.insert(MediaControl::Previous);
    send(
        &mut daemon,
        ServerPayload::MediaUpdated {
            media_session: paused.clone(),
        },
    )
    .await;
    let update = timeout(CALL_TIMEOUT, changed.next())
        .await
        .unwrap()
        .unwrap();
    let (_interface, values, _invalidated): (String, HashMap<String, OwnedValue>, Vec<String>) =
        update.body().deserialize().unwrap();
    assert_eq!(values.len(), 1);
    assert!(bool::try_from(&values["CanGoPrevious"]).unwrap());
    // Position deliberately has no PropertiesChanged signal. Read it afresh
    // rather than using the generic proxy's cached GetAll value.
    let position: OwnedValue = props.call("Get", &(PLAYER, "Position")).await.unwrap();
    assert_eq!(i64::try_from(position).unwrap(), 5_000_000);
    let output = Command::new("playerctl")
        .env("DBUS_SESSION_BUS_ADDRESS", &address)
        .args(["--player=handover.player1", "position"])
        .output()
        .await
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout)
            .unwrap()
            .trim()
            .parse::<f64>()
            .unwrap(),
        5.0
    );

    // Unknown state cannot be projected as Playing, Paused, or Stopped.
    let mut unknown = paused.clone();
    unknown.playback = PlaybackState::Unknown;
    send(
        &mut daemon,
        ServerPayload::MediaUpdated {
            media_session: unknown,
        },
    )
    .await;
    owner(&mut owners, name, false).await;
    send(
        &mut daemon,
        ServerPayload::MediaUpdated {
            media_session: paused.clone(),
        },
    )
    .await;
    let second = "org.mpris.MediaPlayer2.handover.player2";
    owner(&mut owners, second, true).await;
    let mut offline = device();
    offline.connected = false;
    send(
        &mut daemon,
        ServerPayload::DeviceUpdated { device: offline },
    )
    .await;
    owner(&mut owners, second, false).await;
    send(
        &mut daemon,
        ServerPayload::DeviceUpdated { device: device() },
    )
    .await;
    let third = "org.mpris.MediaPlayer2.handover.player3";
    owner(&mut owners, third, true).await;
    drop(daemon);
    owner(&mut owners, third, false).await;
    let _recovered = subscribe(&listener, vec![paused]).await;
    let fourth = "org.mpris.MediaPlayer2.handover.player4";
    owner(&mut owners, fourth, true).await;
    shutdown.send(true).unwrap();
    owner(&mut owners, fourth, false).await;
    timeout(CALL_TIMEOUT, worker).await.unwrap().unwrap();
    timeout(CALL_TIMEOUT, notification_worker)
        .await
        .unwrap()
        .unwrap();
}

#[tokio::test]
async fn playerctl_pause_excludes_the_handover_namespace() {
    let (_bus, address) = private_bus().await;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("daemon.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let (shutdown, receiver) = watch::channel(false);
    let connection = client(&address).await;
    let bus = Proxy::new(
        &connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )
    .await
    .unwrap();
    let mut owners = bus.receive_signal("NameOwnerChanged").await.unwrap();
    let worker = tokio::spawn(run_media(path.clone(), address.clone(), receiver));
    let _daemon = subscribe(&listener, vec![session()]).await;
    owner(&mut owners, "org.mpris.MediaPlayer2.handover.player1", true).await;
    // A normal Linux player shares the test IPC sink, making the selected
    // target observable without launching or controlling a real player.
    let mut local = session();
    local.id.device_id = DeviceId::new("linux-fixture");
    let state = Arc::new(RwLock::new(State {
        session: local.clone(),
        device_name: "Linux".into(),
        active: true,
        revision: 1,
    }));
    let _local_player = zbus::connection::Builder::address(address.as_str())
        .unwrap()
        .name("org.mpris.MediaPlayer2.linuxfixture")
        .unwrap()
        .serve_at(PATH, Root(Arc::clone(&state)))
        .unwrap()
        .serve_at(
            PATH,
            Player {
                state,
                socket: path,
                commands: Arc::new(Semaphore::new(1)),
            },
        )
        .unwrap()
        .build()
        .await
        .unwrap();
    let (output, ()) = tokio::join!(
        Command::new("playerctl")
            .env("DBUS_SESSION_BUS_ADDRESS", &address)
            .args(["--all-players", "--ignore-player=handover", "pause"])
            .output(),
        command(&listener, MediaCommand::Pause { id: local.id }),
    );
    let output = output.expect("playerctl is required for MPRIS integration tests");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        timeout(Duration::from_millis(80), listener.accept())
            .await
            .is_err(),
        "the phone player must not be paused"
    );
    shutdown.send(true).unwrap();
    timeout(CALL_TIMEOUT, worker).await.unwrap().unwrap();
}

mod scoping;
