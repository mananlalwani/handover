mod ipc_server;
mod state;

use std::sync::{Arc, RwLock};
use std::time::Duration;

use handover_core::{DeviceEvent, NotificationEvent, StateEvent};
use handover_kdeconnect::KdeConnectBackend;
use ipc_server::{EVENT_CAPACITY, IpcServer};
use state::{DeviceChange, NotificationChange, StateChange, StateStore};
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::broadcast;
use tracing::{info, warn};

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    info!("handoverd started");

    let state = Arc::new(RwLock::new(StateStore::default()));
    let (events, _) = broadcast::channel(EVENT_CAPACITY);
    let server = match IpcServer::bind(Arc::clone(&state), events.clone()).await {
        Ok(server) => server,
        Err(error) => {
            warn!(%error, "failed to start IPC server");
            return;
        }
    };
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

fn apply_backend_event(
    state: &Arc<RwLock<StateStore>>,
    events: &broadcast::Sender<StateEvent>,
    event: StateEvent,
) {
    let outcome = state
        .write()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .apply(event.clone());
    for change in outcome.changes {
        log_change(change);
    }
    if outcome.changed {
        let _subscriber_count = events.send(event);
    }
}

fn clear_backend_state(state: &Arc<RwLock<StateStore>>, events: &broadcast::Sender<StateEvent>) {
    let snapshot = state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .snapshot();
    for notification in snapshot.notifications {
        apply_backend_event(
            state,
            events,
            StateEvent::Notification(NotificationEvent::Removed(notification.id)),
        );
    }
    for device in snapshot.devices {
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
