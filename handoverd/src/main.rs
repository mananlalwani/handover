mod call_audio;
mod clipboard;
mod ipc_server;
mod messaging;
mod messaging_backend;
mod messaging_cache;
mod presentation;
mod remote_input;
mod screensaver;
mod state;
mod volume;

use std::collections::BTreeSet;
use std::sync::{Mutex, OnceLock};

use std::sync::{Arc, RwLock};
use std::time::Duration;

use handover_core::{
    CallEvent, DeviceEvent, DeviceId, MediaEvent, NotificationEvent, SharedResource, StateEvent,
};
use handover_kdeconnect::KdeConnectBackend;
use handover_native::NativeBackend;
use ipc_server::{EVENT_CAPACITY, IpcServer};
use messaging::MessagingChange;
use messaging_backend::{MessagingHub, spawn_supervisor};
use state::{DeviceChange, MediaChange, NotificationChange, StateChange, StateStore};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::broadcast;
use tracing::{info, warn};

static NATIVE: OnceLock<NativeBackend> = OnceLock::new();
static ACTIVE_CALLS: Mutex<BTreeSet<DeviceId>> = Mutex::new(BTreeSet::new());

pub(crate) fn native_backend() -> Option<&'static NativeBackend> {
    NATIVE.get()
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    info!("handoverd started");

    let mut initial_state = StateStore::default();
    if let Err(error) = messaging_cache::restore(&mut initial_state) {
        warn!(%error, "could not restore messaging cache");
    }
    let state = Arc::new(RwLock::new(initial_state));
    let (events, _) = broadcast::channel(EVENT_CAPACITY);
    match NativeBackend::default_directory().and_then(NativeBackend::open) {
        Ok(native) => {
            for device in native.remembered_devices() {
                apply_backend_event(
                    &state,
                    &events,
                    StateEvent::Device(DeviceEvent::Added(device)),
                );
            }
            let _ = NATIVE.set(native.clone());
            let native_state = Arc::clone(&state);
            let native_events = events.clone();
            std::thread::spawn(move || {
                let callback = Arc::new(move |event| {
                    apply_backend_event(&native_state, &native_events, event)
                });
                if let Err(error) = native.run(callback) {
                    warn!(%error, "native backend stopped");
                }
            });
        }
        Err(error) => warn!(%error, "native backend unavailable"),
    }
    let server = match IpcServer::bind(Arc::clone(&state), events.clone()).await {
        Ok(server) => server,
        Err(error) => {
            warn!(%error, "failed to start IPC server");
            return;
        }
    };
    // Messaging helper supervision is optional and isolated: when no helper
    // binary is configured the subsystem stays dormant and every other
    // backend keeps working. A dead helper only marks its own accounts
    // offline.
    let messaging_hub = MessagingHub::new();
    spawn_supervisor(Arc::clone(&state), events.clone(), messaging_hub.clone());
    let server = server.with_messaging(messaging_hub.clone());
    let backend = run_backend(Arc::clone(&state), events);

    tokio::select! {
        result = server.run() => {
            if let Err(error) = result {
                warn!(%error, "IPC server stopped");
            }
        }
        () = backend => {}
        result = shutdown_signal() => {
            if let Err(error) = result {
                warn!(%error, "failed to listen for shutdown signal");
            }
        }
    }
    // Graceful shutdown also stops the messaging helper so it does not
    // outlive the daemon. An unclean kill can still orphan the helper; the
    // next supervisor generation replaces it on restart.
    messaging_hub.shutdown().await;
    screensaver::update(false);
    tokio::time::sleep(Duration::from_millis(300)).await;
    info!("handoverd stopped");
}

async fn shutdown_signal() -> std::io::Result<()> {
    let mut terminate = signal(SignalKind::terminate())?;

    tokio::select! {
        result = tokio::signal::ctrl_c() => result,
        _ = terminate.recv() => Ok(()),
    }
}

async fn run_backend(state: Arc<RwLock<StateStore>>, events: broadcast::Sender<StateEvent>) {
    loop {
        match KdeConnectBackend::connect().await {
            Ok(backend) => {
                let result = backend
                    .run(|event| apply_backend_event(&state, &events, event))
                    .await;
                if let Err(error) = result {
                    warn!(%error, "KDE Connect backend stopped; reconnecting");
                    clear_backend_state(&state, &events);
                }
            }
            Err(error) => warn!(%error, "could not connect to the session D-Bus; retrying"),
        }

        tokio::time::sleep(Duration::from_secs(5)).await;
    }
}

/// Publish an event that is already reflected in state (used for
/// reconciled windows, where re-applying would suppress the broadcast).
pub(crate) fn publish_event(events: &broadcast::Sender<StateEvent>, event: StateEvent) {
    let _subscriber_count = events.send(event);
}

fn apply_backend_event(
    state: &Arc<RwLock<StateStore>>,
    events: &broadcast::Sender<StateEvent>,
    event: StateEvent,
) {
    if let StateEvent::Presentation(command) = &event {
        presentation::execute(command);
    }
    if let StateEvent::Volume(command) = &event {
        volume::execute(command);
    }
    if let StateEvent::Clipboard(text) = &event {
        clipboard::apply(text);
    }
    if let StateEvent::ClipboardFile(file) = &event {
        clipboard::apply_file(&file.path, &file.mime);
        let _ = std::fs::remove_file(&file.path);
    }
    if let StateEvent::RemoteInput(command) = &event {
        remote_input::execute(command);
    }
    let call_started = match &event {
        StateEvent::Call(CallEvent::Updated(call))
            if call.phase == handover_core::CallPhase::OffHook =>
        {
            ACTIVE_CALLS.lock().unwrap().insert(call.device_id.clone())
        }
        StateEvent::Call(CallEvent::Updated(call)) => {
            ACTIVE_CALLS.lock().unwrap().remove(&call.device_id);
            false
        }
        StateEvent::Call(CallEvent::Removed(id)) => {
            ACTIVE_CALLS.lock().unwrap().remove(id);
            false
        }
        _ => false,
    };
    let media_device = match &event {
        StateEvent::Device(DeviceEvent::Removed(id)) => Some(id),
        StateEvent::Device(DeviceEvent::Updated(device)) if !device.connected || !device.paired => {
            Some(&device.id)
        }
        _ => None,
    };
    if let Some(device_id) = media_device {
        if is_native_device(device_id) {
            apply_backend_event(
                state,
                events,
                StateEvent::Contacts(handover_core::ContactsEvent::Removed(device_id.clone())),
            );
        }
        apply_backend_event(
            state,
            events,
            StateEvent::Call(handover_core::CallEvent::Removed(device_id.clone())),
        );
        let sessions = state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .snapshot()
            .media_sessions;
        for session in sessions
            .into_iter()
            .filter(|session| &session.id.device_id == device_id)
        {
            apply_backend_event(
                state,
                events,
                StateEvent::Media(MediaEvent::Removed(session.id)),
            );
        }
    }
    let messaging_event = matches!(event, StateEvent::Messaging(_));
    let outcome = state
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .apply(event.clone());
    for change in outcome.changes {
        log_change(change);
    }
    if outcome.changed {
        let _subscriber_count = events.send(event);
        if messaging_event && let Err(error) = messaging_cache::persist(state) {
            warn!(%error, "could not persist messaging cache");
        }
    }
    let native_connected = state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .snapshot()
        .devices
        .iter()
        .any(|device| is_native_device(&device.id) && device.connected && device.paired);
    screensaver::update(native_connected);
    if call_started {
        pause_desktop_media();
    }
}

fn pause_desktop_media() {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        handle.spawn(async {
            // playerctl talks to the local MPRIS session, unlike the Handover
            // media map, which describes remote phone players.
            let _ = tokio::process::Command::new("playerctl")
                .args(["--all-players", "pause"])
                .output()
                .await;
        });
    }
}

/// Backend ownership boundary: device IDs namespaced `native:` are owned by
/// the native backend; everything else is KDE-owned. Teardown of one backend
/// must never clear the other's entries. Shares need no filtering: received
/// shares are transient events, never owned snapshot state.
fn is_native_device(id: &DeviceId) -> bool {
    id.as_str().starts_with("native:")
}

fn clear_backend_state(state: &Arc<RwLock<StateStore>>, events: &broadcast::Sender<StateEvent>) {
    let snapshot = state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .snapshot();
    for session in snapshot.media_sessions {
        if is_native_device(&session.id.device_id) {
            continue;
        }
        apply_backend_event(
            state,
            events,
            StateEvent::Media(MediaEvent::Removed(session.id)),
        );
    }
    for notification in snapshot.notifications {
        if is_native_device(&notification.id.device_id) {
            continue;
        }
        apply_backend_event(
            state,
            events,
            StateEvent::Notification(NotificationEvent::Removed(notification.id)),
        );
    }
    for device in snapshot.devices {
        if is_native_device(&device.id) {
            continue;
        }
        apply_backend_event(
            state,
            events,
            StateEvent::Device(DeviceEvent::Removed(device.id)),
        );
    }
}

fn log_change(change: StateChange) {
    match change {
        StateChange::Device(change) => log_device_change(change),
        StateChange::Notification(change) => log_notification_change(change),
        StateChange::Media(change) => log_media_change(change),
        StateChange::Call(call) => {
            tracing::debug!(device_id = %call.device_id, "call state changed")
        }
        StateChange::CallRemoved(id) => tracing::debug!(device_id = %id, "call state unavailable"),
        StateChange::CallCommandResult(result) => tracing::debug!(
            device_id = %result.device_id,
            request_id = %result.request_id,
            action = ?result.action,
            accepted = result.accepted,
            failure = ?result.failure,
            "phone call command result"
        ),
        StateChange::ShareReceived(share) => {
            let kind = match share.resource {
                SharedResource::File { .. } => "file",
                SharedResource::Url { .. } => "url",
            };
            info!(device_id = %share.device_id, kind, "share received");
        }
        StateChange::ShareResult(result) => {
            info!(device_id = %result.device_id, transfer_id = %result.transfer_id,
                status = ?result.status, reason = ?result.reason, "share result");
        }
        StateChange::Messaging(change) => log_messaging_change(change),
    }
}

/// Messaging log fields are ids, counts, and delivery state only. Message
/// bodies, titles, names, addresses, and staged paths never enter logs.
pub(crate) fn log_messaging_change(change: MessagingChange) {
    match change {
        MessagingChange::AccountAdded(id) | MessagingChange::AccountUpdated(id) => {
            info!(account_id = %id, "messaging account updated")
        }
        MessagingChange::AccountRemoved(id) => {
            info!(account_id = %id, "messaging account removed")
        }
        MessagingChange::ConversationAdded(id) | MessagingChange::ConversationUpdated(id) => {
            info!(conversation_id = %id, "conversation updated")
        }
        MessagingChange::ConversationRemoved(id) => {
            info!(conversation_id = %id, "conversation removed")
        }
        MessagingChange::MessageAdded(id) => {
            info!(message_id = %id, "message added")
        }
        MessagingChange::MessageUpdated(id) => {
            tracing::debug!(message_id = %id, "message updated")
        }
        MessagingChange::MessageRemoved(id) => {
            info!(message_id = %id, "message removed")
        }
        MessagingChange::Status(update) => {
            info!(message_id = %update.message_id, status = ?update.status, "message status")
        }
        MessagingChange::Typing(id) => {
            tracing::debug!(conversation_id = %id, "typing state")
        }
        MessagingChange::TypingCleared(id) => {
            tracing::debug!(conversation_id = %id, "typing cleared")
        }
        MessagingChange::Read(id) => {
            tracing::debug!(conversation_id = %id, "read state")
        }
        MessagingChange::Pairing(id) => {
            info!(account_id = %id, "pairing verification requested")
        }
    }
}

fn log_media_change(change: MediaChange) {
    match change {
        MediaChange::Added(id) => {
            info!(device_id = %id.device_id, player_id = %id.player_id, "media session added")
        }
        MediaChange::Updated(id) => {
            tracing::debug!(device_id = %id.device_id, player_id = %id.player_id, "media session updated")
        }
        MediaChange::Removed(id) => {
            info!(device_id = %id.device_id, player_id = %id.player_id, "media session removed")
        }
    }
}

fn log_device_change(change: DeviceChange) {
    match change {
        DeviceChange::Discovered { id, name, paired } => {
            info!(device_id = %id, device_name = name, paired, "device discovered");
        }
        DeviceChange::NameChanged { id, old, new } => {
            info!(device_id = %id, old_name = old, device_name = new, "device name changed");
        }
        DeviceChange::Connected { id, name } => {
            info!(device_id = %id, device_name = name, "device connected");
        }
        DeviceChange::Disconnected { id, name } => {
            info!(device_id = %id, device_name = name, "device disconnected");
        }
        DeviceChange::PairingChanged { id, name, paired } => {
            info!(device_id = %id, device_name = name, paired, "device pairing changed");
        }
        DeviceChange::BatteryChanged { id, name, battery } => match battery {
            Some(battery) => info!(
                device_id = %id,
                device_name = name,
                percentage = battery.percentage(),
                charging = battery.charging,
                "battery updated"
            ),
            None => info!(device_id = %id, device_name = name, "battery unavailable"),
        },
        DeviceChange::Removed { id, name } => {
            info!(device_id = %id, device_name = name, "device removed");
        }
    }
}

fn log_notification_change(change: NotificationChange) {
    match change {
        NotificationChange::Added(notification) => info!(
            notification_id = %notification.id,
            device_id = %notification.id.device_id,
            app_name = notification.app_name,
            "notification added"
        ),
        NotificationChange::Updated(notification) => info!(
            notification_id = %notification.id,
            device_id = %notification.id.device_id,
            app_name = notification.app_name,
            "notification updated"
        ),
        NotificationChange::Removed { id, app_name } => info!(
            notification_id = %id,
            device_id = %id.device_id,
            app_name,
            "notification removed"
        ),
    }
}

#[cfg(test)]
mod native_coexistence_tests {
    use super::*;
    use handover_core::{
        Capability, Device, DeviceId, MediaSession, MediaSessionId, Notification, NotificationId,
        PlaybackState,
    };
    use std::collections::BTreeSet;

    fn device(id: &str, capabilities: BTreeSet<Capability>) -> Device {
        Device {
            id: DeviceId::new(id),
            name: id.into(),
            connected: true,
            paired: true,
            battery: None,
            connectivity: None,
            capabilities,
        }
    }

    fn notification(device_id: &str, local_id: &str) -> Notification {
        Notification {
            id: NotificationId::new(DeviceId::new(device_id), local_id),
            app_name: "Example".into(),
            title: "Hello".into(),
            body: "World".into(),
            icon_path: None,
            clearable: true,
            actions: vec![],
            reply_supported: false,
        }
    }

    fn media_session(device_id: &str, player_id: &str) -> MediaSession {
        MediaSession {
            id: MediaSessionId::new(DeviceId::new(device_id), player_id),
            application: "Example Music".into(),
            title: Some("Test track".into()),
            artist: None,
            album: None,
            playback: PlaybackState::Playing,
            position_ms: None,
            duration_ms: None,
            volume_percent: None,
            controls: BTreeSet::from([handover_core::MediaControl::Pause]),
        }
    }

    fn populated_state() -> (Arc<RwLock<StateStore>>, broadcast::Sender<StateEvent>) {
        let state = Arc::new(RwLock::new(StateStore::default()));
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        for (id, capabilities) in [
            ("kde-device", BTreeSet::from([Capability::Notifications])),
            (
                "native:cert",
                BTreeSet::from([Capability::Notifications, Capability::Media]),
            ),
        ] {
            apply_backend_event(
                &state,
                &events,
                StateEvent::Device(DeviceEvent::Added(device(id, capabilities))),
            );
            apply_backend_event(
                &state,
                &events,
                StateEvent::Notification(NotificationEvent::Added(notification(id, "n1"))),
            );
            apply_backend_event(
                &state,
                &events,
                StateEvent::Media(MediaEvent::Added(media_session(id, "player"))),
            );
        }
        (state, events)
    }

    #[test]
    fn kde_loss_keeps_native_device() {
        let state = Arc::new(RwLock::new(StateStore::default()));
        let (events, _) = broadcast::channel(EVENT_CAPACITY);
        for id in ["kde-device", "native:cert"] {
            apply_backend_event(
                &state,
                &events,
                StateEvent::Device(DeviceEvent::Added(Device {
                    id: DeviceId::new(id),
                    name: id.into(),
                    connected: true,
                    paired: true,
                    battery: None,
                    connectivity: None,
                    capabilities: [Capability::Battery].into(),
                })),
            );
        }
        clear_backend_state(&state, &events);
        let devices = state.read().unwrap().snapshot().devices;
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].id.as_str(), "native:cert");
    }

    #[test]
    fn kde_teardown_clears_only_kde_owned_state() {
        let (state, events) = populated_state();
        clear_backend_state(&state, &events);

        let snapshot = state.read().unwrap().snapshot();
        assert_eq!(
            snapshot
                .devices
                .iter()
                .map(|device| device.id.as_str())
                .collect::<Vec<_>>(),
            ["native:cert"]
        );
        assert_eq!(
            snapshot
                .notifications
                .iter()
                .map(|notification| notification.id.to_string())
                .collect::<Vec<_>>(),
            ["native:cert:n1"]
        );
        assert_eq!(
            snapshot
                .media_sessions
                .iter()
                .map(|session| session.id.player_id.clone())
                .collect::<Vec<_>>(),
            ["player"]
        );
        assert_eq!(
            snapshot.media_sessions[0].id.device_id.as_str(),
            "native:cert"
        );
    }

    #[test]
    fn native_teardown_clears_only_native_owned_state() {
        let (state, events) = populated_state();
        // Mirror the native session-drop event sequence: per-peer removals
        // followed by the device going offline.
        for event in [
            StateEvent::Notification(NotificationEvent::Removed(NotificationId::new(
                DeviceId::new("native:cert"),
                "n1",
            ))),
            StateEvent::Media(MediaEvent::Removed(MediaSessionId::new(
                DeviceId::new("native:cert"),
                "player",
            ))),
            StateEvent::Device(DeviceEvent::Removed(DeviceId::new("native:cert"))),
        ] {
            apply_backend_event(&state, &events, event);
        }

        let snapshot = state.read().unwrap().snapshot();
        assert_eq!(
            snapshot
                .devices
                .iter()
                .map(|device| device.id.as_str())
                .collect::<Vec<_>>(),
            ["kde-device"]
        );
        assert_eq!(
            snapshot
                .notifications
                .iter()
                .map(|notification| notification.id.to_string())
                .collect::<Vec<_>>(),
            ["kde-device:n1"]
        );
        assert_eq!(
            snapshot
                .media_sessions
                .iter()
                .map(|session| session.id.player_id.clone())
                .collect::<Vec<_>>(),
            ["player"]
        );
        assert_eq!(
            snapshot.media_sessions[0].id.device_id.as_str(),
            "kde-device"
        );
    }
}
