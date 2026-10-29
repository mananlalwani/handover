//! Disposable Linux desktop integration client. The daemon remains the source of truth.
mod media;
mod media_player;
mod model;
pub use media::run as run_media;

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use futures_util::{StreamExt, stream};
use handover_core::{Notification, NotificationCommand, NotificationId};
use handover_ipc::{Client, ServerPayload};
use model::{Entry, MAX_NOTIFICATIONS, Model};
use tokio::sync::{mpsc, watch};
use tokio::time::timeout;
use tracing::warn;
use zbus::{Connection, Proxy, fdo::DBusProxy, zvariant::Value};

const SERVICE: &str = "org.freedesktop.Notifications";
const PATH: &str = "/org/freedesktop/Notifications";
const CALL_TIMEOUT: Duration = Duration::from_secs(3);
type Error = Box<dyn std::error::Error + Send + Sync>;

/// Runs until shutdown. Reconnects from current state, never replaying commands.
pub async fn run(path: PathBuf, connection: Connection, mut shutdown: watch::Receiver<bool>) {
    while !*shutdown.borrow() {
        if session(&path, &connection, &mut shutdown).await.is_err() {
            // Transport errors can contain private notification fields. Keep diagnostics fixed.
            warn!("desktop notification session ended; reconnecting");
        }
        if *shutdown.borrow() {
            break;
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_secs(2)) => {}
            changed = shutdown.changed() => { if changed.is_err() || *shutdown.borrow() { break; } }
        }
    }
}

enum Update {
    Snapshot(Vec<Notification>),
    Post(Notification),
    Remove(NotificationId),
}

// Keep framing futures alive independently of D-Bus select branches. IPC reads
// consume partial lines and must not be cancelled while the socket stays open.
struct Reader(tokio::task::JoinHandle<()>);
impl Drop for Reader {
    fn drop(&mut self) {
        self.0.abort();
    }
}

async fn session(
    path: &Path,
    connection: &Connection,
    shutdown: &mut watch::Receiver<bool>,
) -> Result<(), Error> {
    let bus = DBusProxy::new(connection).await?;
    let mut owner_changes = bus
        .receive_name_owner_changed_with_args(&[(0, SERVICE)])
        .await?;
    let owner = loop {
        match timeout(CALL_TIMEOUT, bus.get_name_owner(SERVICE.try_into()?)).await? {
            Ok(owner) => break owner,
            Err(zbus::fdo::Error::NameHasNoOwner(_)) => {
                // Media integration also runs on desktops without a notification
                // service. Wait for an owner instead of repeatedly reconnecting.
                tokio::select! {
                    change = owner_changes.next() => { change.ok_or("session bus disconnected")?; }
                    changed = shutdown.changed() => {
                        if changed.is_err() || *shutdown.borrow() { return Ok(()); }
                    }
                }
            }
            Err(error) => return Err(error.into()),
        }
    };
    // Pin the unique owner. IDs from a previous notification service must never
    // be sent to its replacement, where those IDs could identify unrelated apps.
    let proxy = Proxy::new(connection, owner, PATH, SERVICE).await?;
    let mut signals = proxy.receive_all_signals().await?;
    let capabilities: Vec<String> =
        timeout(CALL_TIMEOUT, proxy.call("GetCapabilities", &())).await??;
    let markup = capabilities.iter().any(|item| item == "body-markup");
    let subscription = timeout(CALL_TIMEOUT, async {
        Client::connect_to(path.to_path_buf())
            .await?
            .subscribe_notifications()
            .await
    })
    .await??;
    let (sender, mut receiver) = mpsc::channel(8);
    let _reader = Reader(tokio::spawn(async move {
        let mut subscription = subscription;
        if sender
            .send(Update::Snapshot(std::mem::take(
                &mut subscription.notifications,
            )))
            .await
            .is_err()
        {
            return;
        }
        while let Ok(message) = subscription.next_message().await {
            let update = match message.payload {
                ServerPayload::Snapshot { notifications, .. } => Update::Snapshot(notifications),
                ServerPayload::NotificationAdded { notification }
                | ServerPayload::NotificationUpdated { notification } => Update::Post(notification),
                ServerPayload::NotificationRemoved { notification_id } => {
                    Update::Remove(notification_id)
                }
                _ => continue,
            };
            if sender.send(update).await.is_err() {
                return;
            }
        }
    }));
    let mut model = Model::default();
    let outcome = async {
        loop {
            tokio::select! {
                update = receiver.recv() => match update {
                    Some(Update::Snapshot(notifications)) => {
                        // Refresh existing visible entries quietly. New baseline entries
                        // are remembered without generating a backlog of old alerts.
                        for notification in &notifications {
                            if model.entries.get(&notification.id).is_some_and(|entry| entry.desktop_id.is_some() && entry.notification != *notification) {
                                post(&proxy, &mut model, notification.clone(), markup, true).await?;
                            }
                        }
                        for id in model.snapshot(notifications) { close(&proxy, id).await; }
                    }
                    Some(Update::Post(notification)) => post(&proxy, &mut model, notification, markup, false).await?,
                    Some(Update::Remove(id)) => {
                        if let Some(desktop_id) = model.remove(&id) { close(&proxy, desktop_id).await; }
                    }
                    None => return Err::<(), Error>("daemon disconnected".into()),
                },
                signal = signals.next() => {
                    let signal = signal.ok_or("notification service disconnected")?;
                    match signal.header().member().map(|member| member.as_str()) {
                        Some("NotificationClosed") => {
                            let (id, _reason): (u32, u32) = signal.body().deserialize()?;
                            model.closed(id);
                        }
                        Some("ActionInvoked") => {
                            let (id, key): (u32, String) = signal.body().deserialize()?;
                            if let Some(command) = model.action(id, &key) {
                                // A timeout is ambiguous. Do not retry a remote action.
                                if !matches!(timeout(CALL_TIMEOUT, execute(path.to_path_buf(), command)).await, Ok(Ok(()))) {
                                    warn!("desktop notification action was not confirmed by the daemon");
                                }
                            }
                        }
                        _ => {}
                    }
                }
                _ = owner_changes.next() => return Err("notification service owner changed".into()),
                changed = shutdown.changed() => {
                    if changed.is_err() || *shutdown.borrow() { return Ok(()); }
                }
            }
        }
    }.await;
    // Bound shutdown time, including when the notification service is wedged.
    let ids: Vec<_> = model
        .entries
        .values()
        .filter_map(|entry| entry.desktop_id)
        .collect();
    let _ = timeout(
        Duration::from_secs(2),
        stream::iter(ids).for_each_concurrent(8, |id| close(&proxy, id)),
    )
    .await;
    outcome
}

async fn execute(path: PathBuf, command: NotificationCommand) -> Result<(), Error> {
    let mut client = Client::connect_to(path).await?;
    match command {
        NotificationCommand::Dismiss { notification_id } => {
            client.dismiss_notification(notification_id).await?
        }
        NotificationCommand::InvokeAction {
            notification_id,
            action_id,
        } => {
            client
                .invoke_notification_action(notification_id, action_id)
                .await?
        }
        NotificationCommand::Reply { .. } => return Err("inline reply is unsupported".into()),
    }
    Ok(())
}

async fn post(
    proxy: &Proxy<'_>,
    model: &mut Model,
    notification: Notification,
    markup: bool,
    quiet: bool,
) -> Result<(), Error> {
    let previous = model.entries.get(&notification.id);
    if previous.is_some_and(|entry| entry.notification == notification) {
        return Ok(());
    }
    if previous.is_none() && model.entries.len() >= MAX_NOTIFICATIONS {
        return Ok(());
    }
    let replaces_id = previous.and_then(|entry| entry.desktop_id).unwrap_or(0);
    let mut hints = HashMap::<&str, Value<'_>>::new();
    hints.insert("desktop-entry", "handover".into());
    hints.insert("resident", true.into());
    hints.insert("suppress-sound", quiet.into());
    let desktop_id: u32 = timeout(
        CALL_TIMEOUT,
        proxy.call(
            "Notify",
            &(
                format!("Handover · {}", notification.app_name),
                replaces_id,
                "network-wireless",
                &notification.title,
                model::body(&notification.body, markup),
                model::actions(&notification),
                hints,
                -1i32,
            ),
        ),
    )
    .await??;
    if desktop_id == 0 {
        return Err("notification service returned an invalid ID".into());
    }
    model.entries.insert(
        notification.id.clone(),
        Entry {
            notification,
            desktop_id: Some(desktop_id),
        },
    );
    Ok(())
}

async fn close(proxy: &Proxy<'_>, id: u32) {
    let _ = timeout(
        CALL_TIMEOUT,
        proxy.call::<_, _, ()>("CloseNotification", &(id,)),
    )
    .await;
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod integration;

#[cfg(test)]
mod media_integration;
