use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::{CommandFactory, Parser, Subcommand};
use handover_core::{
    Device, DeviceId, MediaCommand, MediaSession, MediaSessionId, Notification, SharedResource,
};
use handover_ipc::{Client, IpcError, PROTOCOL_VERSION, ServerPayload};
use thiserror::Error;
use url::Url;

#[derive(Debug, Parser)]
#[command(about = "Inspect and control Handover", version)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// List devices known to handoverd
    Devices,
    /// Inspect and manage native Android pairing
    Native {
        #[command(subcommand)]
        command: NativeCommand,
    },
    /// List active remote notifications
    Notifications,
    /// List and control active remote media sessions
    Media {
        #[command(subcommand)]
        command: Option<MediaSubcommand>,
    },
    /// Print live normalized device and notification changes
    Monitor,
    /// Send a URL to one paired device
    SendUrl { device: String, url: String },
    /// Send one local file to a paired device
    SendFile { device: String, path: PathBuf },
}

#[derive(Debug, Subcommand)]
enum NativeCommand {
    Peers,
    Pending,
    Pair { id: String, code: String },
    Unpair { id: String },
}

#[derive(Debug, Subcommand)]
enum MediaSubcommand {
    /// Start playback
    Play { session: String },
    /// Pause playback
    Pause { session: String },
    /// Toggle playback
    #[command(name = "play-pause")]
    PlayPause { session: String },
    /// Skip to the next item
    Next { session: String },
    /// Return to the previous item
    Previous { session: String },
}

#[derive(Debug, Error)]
enum CliError {
    #[error(transparent)]
    Ipc(#[from] IpcError),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    DeviceSelection(String),
    #[error("{0}")]
    MediaSelection(String),
    #[error("cannot convert local path to a file URL")]
    InvalidFilePath,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Some(Command::Devices) => list_devices().await,
        Some(Command::Native { command }) => native(command).await,
        Some(Command::Notifications) => list_notifications().await,
        Some(Command::Media { command }) => media(command).await,
        Some(Command::Monitor) => monitor().await,
        Some(Command::SendUrl { device, url }) => send_url(&device, url).await,
        Some(Command::SendFile { device, path }) => send_file(&device, path).await,
        None => {
            Cli::command()
                .print_help()
                .expect("writing help to stdout should succeed");
            println!();
            return ExitCode::SUCCESS;
        }
    };

    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("handoverctl: {error}");
            ExitCode::FAILURE
        }
    }
}

async fn native(command: NativeCommand) -> Result<(), CliError> {
    let mut client = connected_client().await?;
    match command {
        NativeCommand::Peers => {
            for peer in client.native_peers().await? {
                println!("{}\t{}\t{}", peer.id, peer.name, peer.fingerprint);
            }
        }
        NativeCommand::Pending => {
            for peer in client.native_pending().await? {
                println!("{}\t{}\t{}", peer.id, peer.name, peer.code);
            }
        }
        NativeCommand::Pair { id, code } => {
            client.native_pair(id, code).await?;
            println!("Pair approval recorded; waiting for phone confirmation");
        }
        NativeCommand::Unpair { id } => {
            client.native_unpair(id).await?;
            println!("Native peer revoked");
        }
    }
    Ok(())
}

async fn list_devices() -> Result<(), CliError> {
    let mut client = connected_client().await?;
    let devices = client.devices().await?;
    print_table(&devices);
    Ok(())
}

async fn list_notifications() -> Result<(), CliError> {
    let mut client = connected_client().await?;
    let devices = client.devices().await?;
    let notifications = client.notifications().await?;
    print_notification_table(&devices, &notifications);
    Ok(())
}

async fn media(command: Option<MediaSubcommand>) -> Result<(), CliError> {
    let mut client = connected_client().await?;
    let devices = client.devices().await?;
    let sessions = client.media_sessions().await?;

    let Some(command) = command else {
        print_media_table(&devices, &sessions);
        return Ok(());
    };

    let (selector, command) = match command {
        MediaSubcommand::Play { session } => (session, MediaAction::Play),
        MediaSubcommand::Pause { session } => (session, MediaAction::Pause),
        MediaSubcommand::PlayPause { session } => (session, MediaAction::PlayPause),
        MediaSubcommand::Next { session } => (session, MediaAction::Next),
        MediaSubcommand::Previous { session } => (session, MediaAction::Previous),
    };
    let id = select_media_session(&sessions, &selector)?;
    client
        .media_command(command.into_command(id.clone()))
        .await?;
    println!("media command accepted for {}", media_selector(&id));
    Ok(())
}

#[derive(Clone, Copy)]
enum MediaAction {
    Play,
    Pause,
    PlayPause,
    Next,
    Previous,
}

impl MediaAction {
    fn into_command(self, id: MediaSessionId) -> MediaCommand {
        match self {
            Self::Play => MediaCommand::Play { id },
            Self::Pause => MediaCommand::Pause { id },
            Self::PlayPause => MediaCommand::PlayPause { id },
            Self::Next => MediaCommand::Next { id },
            Self::Previous => MediaCommand::Previous { id },
        }
    }
}

async fn monitor() -> Result<(), CliError> {
    loop {
        match connected_client().await {
            Ok(client) => match client.subscribe().await {
                Ok(mut subscription) => {
                    println!(
                        "subscribed: {} device(s), {} notification(s)",
                        subscription.devices.len(),
                        subscription.notifications.len()
                    );
                    loop {
                        tokio::select! {
                            message = subscription.next_message() => {
                                match message {
                                    Ok(message) => print_message(message.payload),
                                    Err(error) => {
                                        eprintln!("handoverctl: connection lost: {error}; retrying");
                                        break;
                                    }
                                }
                            }
                            result = tokio::signal::ctrl_c() => {
                                result.map_err(IpcError::Io)?;
                                return Ok(());
                            }
                        }
                    }
                }
                Err(error) => eprintln!("handoverctl: subscription failed: {error}; retrying"),
            },
            Err(error) => eprintln!("handoverctl: daemon unavailable: {error}; retrying"),
        }

        tokio::select! {
            () = tokio::time::sleep(Duration::from_secs(2)) => {}
            result = tokio::signal::ctrl_c() => {
                result.map_err(IpcError::Io)?;
                return Ok(());
            }
        }
    }
}

async fn send_url(selector: &str, url: String) -> Result<(), CliError> {
    let mut client = connected_client().await?;
    let device_id = select_device(&client.devices().await?, selector)?;
    client.send_url(device_id, url).await?;
    println!("URL share accepted; delivery is not confirmed");
    Ok(())
}

async fn send_file(selector: &str, path: PathBuf) -> Result<(), CliError> {
    let mut client = connected_client().await?;
    let device_id = select_device(&client.devices().await?, selector)?;
    let absolute = tokio::fs::canonicalize(path).await?;
    let file_url = Url::from_file_path(absolute).map_err(|_| CliError::InvalidFilePath)?;
    client.send_file_url(device_id, file_url.into()).await?;
    println!("File share accepted; delivery is not confirmed");
    Ok(())
}

fn select_device(devices: &[Device], selector: &str) -> Result<DeviceId, CliError> {
    if let Some(device) = devices.iter().find(|device| device.id.as_str() == selector) {
        return Ok(device.id.clone());
    }
    let mut matches = devices.iter().filter(|device| device.name == selector);
    let first = matches.next().ok_or_else(|| {
        CliError::DeviceSelection(format!("no device named or identified by {selector:?}"))
    })?;
    if matches.next().is_some() {
        return Err(CliError::DeviceSelection(format!(
            "multiple devices are named {selector:?}; use a device ID"
        )));
    }
    Ok(first.id.clone())
}

async fn connected_client() -> Result<Client, IpcError> {
    let mut client = Client::connect().await?;
    let supported = client.hello().await?;
    if !supported.contains(&PROTOCOL_VERSION) {
        return Err(IpcError::UnexpectedResponse(format!(
            "daemon does not support protocol {PROTOCOL_VERSION}"
        )));
    }
    Ok(client)
}

fn print_table(devices: &[Device]) {
    let name_width = devices
        .iter()
        .map(|device| device.name.chars().count())
        .max()
        .unwrap_or(6)
        .max(6);
    println!(
        "{:<name_width$}  {:<12}  {:<8}  BATTERY",
        "DEVICE", "STATUS", "PAIRED"
    );
    for device in devices {
        let status = if device.connected {
            "connected"
        } else {
            "offline"
        };
        let paired = if device.paired { "yes" } else { "no" };
        let battery = device
            .battery
            .map(|battery| format!("{}%", battery.percentage()))
            .unwrap_or_else(|| "-".into());
        println!(
            "{:<name_width$}  {:<12}  {:<8}  {}",
            device.name, status, paired, battery
        );
    }
}

fn print_notification_table(devices: &[Device], notifications: &[Notification]) {
    println!("DEVICE  APP  TITLE");
    for notification in notifications {
        let device_name = devices
            .iter()
            .find(|device| device.id == notification.id.device_id)
            .map(|device| device.name.as_str())
            .unwrap_or_else(|| notification.id.device_id.as_str());
        println!(
            "{}  {}  {}",
            device_name, notification.app_name, notification.title
        );
    }
}

fn print_message(payload: ServerPayload) {
    match payload {
        ServerPayload::DeviceAdded { device } => {
            println!("device added: {}", describe_device(&device));
        }
        ServerPayload::DeviceUpdated { device } => {
            println!("device updated: {}", describe_device(&device));
        }
        ServerPayload::DeviceRemoved { device_id } => {
            println!("device removed: {device_id}");
        }
        ServerPayload::NotificationAdded { notification } => {
            println!(
                "notification added: {} — {}",
                notification.app_name, notification.title
            );
        }
        ServerPayload::NotificationUpdated { notification } => {
            println!(
                "notification updated: {} — {}",
                notification.app_name, notification.title
            );
        }
        ServerPayload::NotificationRemoved { notification_id } => {
            println!("notification removed: {notification_id}");
        }
        ServerPayload::MediaAdded { media_session } => {
            println!("media added: {}", describe_media(&media_session));
        }
        ServerPayload::MediaUpdated { media_session } => {
            println!("media updated: {}", describe_media(&media_session));
        }
        ServerPayload::MediaRemoved { media_session_id } => {
            println!("media removed: {}", media_selector(&media_session_id));
        }
        ServerPayload::ShareReceived { share } => {
            let kind = match share.resource {
                SharedResource::File { .. } => "local file available",
                SharedResource::Url { .. } => "URL",
            };
            println!("share received: {kind} from {}", share.device_id);
        }
        ServerPayload::Snapshot {
            devices,
            notifications,
            media_sessions,
        } => {
            println!(
                "state resynchronized: {} device(s), {} notification(s), {} media session(s)",
                devices.len(),
                notifications.len(),
                media_sessions.len()
            );
        }
        ServerPayload::Error { code, message } => {
            eprintln!("daemon error ({code:?}): {message}");
        }
        ServerPayload::Hello { .. }
        | ServerPayload::Devices { .. }
        | ServerPayload::Notifications { .. }
        | ServerPayload::Media { .. }
        | ServerPayload::CommandCompleted { .. }
        | ServerPayload::ShareAccepted { .. }
        | ServerPayload::MediaAccepted { .. }
        | ServerPayload::Subscribed { .. }
        | ServerPayload::NativePeers { .. }
        | ServerPayload::NativePending { .. }
        | ServerPayload::NativeAccepted => {}
    }
}

fn print_media_table(devices: &[Device], sessions: &[MediaSession]) {
    println!("SESSION  DEVICE  APP  STATE  TITLE  ARTIST");
    for session in sessions {
        let device_name = devices
            .iter()
            .find(|device| device.id == session.id.device_id)
            .map(|device| device.name.as_str())
            .unwrap_or_else(|| session.id.device_id.as_str());
        println!(
            "{}  {}  {}  {}  {}  {}",
            media_selector(&session.id),
            device_name,
            session.application,
            playback_label(session),
            session.title.as_deref().unwrap_or("-"),
            session.artist.as_deref().unwrap_or("-")
        );
    }
}

fn playback_label(session: &MediaSession) -> &'static str {
    match session.playback {
        handover_core::PlaybackState::Playing => "playing",
        handover_core::PlaybackState::Paused => "paused",
        handover_core::PlaybackState::Stopped => "stopped",
        handover_core::PlaybackState::Unknown => "unknown",
    }
}

fn media_selector(id: &MediaSessionId) -> String {
    format!("{}:{}", id.device_id, id.player_id)
}

fn select_media_session(
    sessions: &[MediaSession],
    selector: &str,
) -> Result<MediaSessionId, CliError> {
    if let Some(session) = sessions
        .iter()
        .find(|session| media_selector(&session.id) == selector)
    {
        return Ok(session.id.clone());
    }

    let matches: Vec<_> = sessions
        .iter()
        .filter(|session| session.application == selector)
        .collect();
    match matches.as_slice() {
        [session] => Ok(session.id.clone()),
        [] => Err(CliError::MediaSelection(format!(
            "no media session identified by {selector:?}"
        ))),
        _ => Err(CliError::MediaSelection(format!(
            "multiple media sessions use application {selector:?}; use a session ID"
        ))),
    }
}

fn describe_media(session: &MediaSession) -> String {
    format!(
        "{} {} state={} title={}",
        media_selector(&session.id),
        session.application,
        playback_label(session),
        session.title.as_deref().unwrap_or("-")
    )
}

fn describe_device(device: &Device) -> String {
    let battery = device
        .battery
        .map(|battery| format!("{}% charging={}", battery.percentage(), battery.charging))
        .unwrap_or_else(|| "unavailable".into());
    format!(
        "{} id={} connected={} paired={} battery={}",
        device.name, device.id, device.connected, device.paired, battery
    )
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use handover_core::{BatteryState, Capability, DeviceId, MediaSession, PlaybackState};

    use super::*;

    #[test]
    fn describes_normalized_device() {
        let device = Device {
            id: DeviceId::new("phone-123"),
            name: "Phone".into(),
            connected: true,
            paired: true,
            battery: Some(BatteryState::new(55, false).expect("valid battery")),
            capabilities: BTreeSet::from([Capability::Battery]),
        };

        assert_eq!(
            describe_device(&device),
            "Phone id=phone-123 connected=true paired=true battery=55% charging=false"
        );
    }

    #[test]
    fn selects_id_or_unique_exact_name_without_guessing() {
        let mut first = Device {
            id: DeviceId::new("phone-a"),
            name: "Phone".into(),
            connected: true,
            paired: true,
            battery: None,
            capabilities: BTreeSet::new(),
        };
        let second = Device {
            id: DeviceId::new("phone-b"),
            name: "Tablet".into(),
            ..first.clone()
        };
        assert_eq!(
            select_device(&[first.clone(), second.clone()], "Tablet").expect("unique name"),
            second.id
        );
        assert_eq!(
            select_device(&[first.clone(), second], "phone-a").expect("ID"),
            first.id
        );
        first.id = DeviceId::new("phone-c");
        assert!(matches!(
            select_device(&[first.clone(), Device { id: DeviceId::new("phone-a"), ..first }], "Phone"),
            Err(CliError::DeviceSelection(message)) if message.contains("multiple")
        ));
        assert!(matches!(
            select_device(&[], "missing"),
            Err(CliError::DeviceSelection(message)) if message.contains("no device")
        ));
    }

    fn media_session(device_id: &str, player_id: &str, application: &str) -> MediaSession {
        MediaSession {
            id: MediaSessionId::new(DeviceId::new(device_id), player_id),
            application: application.into(),
            title: Some("Song".into()),
            artist: Some("Artist".into()),
            album: None,
            playback: PlaybackState::Playing,
            position_ms: None,
            duration_ms: None,
            volume_percent: None,
            controls: Default::default(),
        }
    }

    #[test]
    fn selects_media_by_exact_scoped_id_or_unique_application() {
        let first = media_session("phone-a", "player-1", "Spotify");
        let second = media_session("phone-b", "player-2", "Music");
        assert_eq!(
            select_media_session(&[first.clone(), second.clone()], "phone-a:player-1")
                .expect("scoped ID"),
            first.id
        );
        assert_eq!(
            select_media_session(&[first.clone(), second.clone()], "Music").expect("unique app"),
            second.id
        );
    }

    #[test]
    fn rejects_ambiguous_or_unknown_media_selector() {
        let first = media_session("phone-a", "player-1", "Spotify");
        let second = media_session("phone-b", "player-2", "Spotify");
        assert!(matches!(
            select_media_session(&[first.clone(), second], "Spotify"),
            Err(CliError::MediaSelection(message)) if message.contains("multiple")
        ));
        assert!(matches!(
            select_media_session(&[first], "missing"),
            Err(CliError::MediaSelection(message)) if message.contains("no media session")
        ));
    }

    #[test]
    fn formats_media_state_without_optional_metadata() {
        let mut session = media_session("phone-a", "player-1", "Spotify");
        session.title = None;
        session.playback = PlaybackState::Paused;
        assert_eq!(
            describe_media(&session),
            "phone-a:player-1 Spotify state=paused title=-"
        );
    }
}
