//! KDE Connect D-Bus adapter for Handover.
//!
//! All KDE Connect service names, object paths, interfaces, properties, and
//! signal formats stay in this crate. Consumers receive normalized
//! [`handover_core::StateEvent`] values.

use std::collections::{BTreeMap, BTreeSet};

use futures_util::StreamExt;
use handover_core::{
    BatteryState, Capability, Device, DeviceEvent, DeviceId, Notification, NotificationCommand,
    NotificationEvent, NotificationId, ReceivedShare, SharedResource, StateEvent,
};
use thiserror::Error;
use tracing::{debug, info, warn};
use url::Url;
use zbus::fdo::DBusProxy;
use zbus::message::Type;
use zbus::names::{BusName, WellKnownName};
use zbus::{Connection, MatchRule, Message, MessageStream};

const SERVICE: &str = "org.kde.kdeconnect";
const DAEMON_PATH: &str = "/modules/kdeconnect";
const DEVICES_PATH: &str = "/modules/kdeconnect/devices";
const BATTERY_PLUGIN: &str = "kdeconnect_battery";
const NOTIFICATIONS_PLUGIN: &str = "kdeconnect_notifications";
const SHARE_PLUGIN: &str = "kdeconnect_share";

#[zbus::proxy(
    default_service = "org.kde.kdeconnect",
    default_path = "/modules/kdeconnect",
    interface = "org.kde.kdeconnect.daemon"
)]
trait Daemon {
    #[zbus(name = "devices")]
    fn devices(&self, only_reachable: bool, only_paired: bool) -> zbus::Result<Vec<String>>;
}

#[zbus::proxy(interface = "org.kde.kdeconnect.device")]
trait KdeDevice {
    #[zbus(property, name = "name")]
    fn name(&self) -> zbus::Result<String>;

    #[zbus(property, name = "isReachable")]
    fn is_reachable(&self) -> zbus::Result<bool>;

    #[zbus(property, name = "isPaired")]
    fn is_paired(&self) -> zbus::Result<bool>;

    #[zbus(property, name = "supportedPlugins")]
    fn supported_plugins(&self) -> zbus::Result<Vec<String>>;

    #[zbus(name = "isPluginEnabled")]
    fn is_plugin_enabled(&self, plugin: &str) -> zbus::Result<bool>;
}

#[zbus::proxy(interface = "org.kde.kdeconnect.device.share")]
trait Share {
    #[zbus(name = "shareUrl")]
    fn share_url(&self, url: &str) -> zbus::Result<()>;
}

#[zbus::proxy(interface = "org.kde.kdeconnect.device.battery")]
trait Battery {
    #[zbus(property, name = "charge")]
    fn charge(&self) -> zbus::Result<i32>;

    #[zbus(property, name = "isCharging")]
    fn is_charging(&self) -> zbus::Result<bool>;

    #[zbus(property, name = "hasBattery")]
    fn has_battery(&self) -> zbus::Result<bool>;
}

#[zbus::proxy(interface = "org.kde.kdeconnect.device.notifications")]
trait Notifications {
    #[zbus(name = "activeNotifications")]
    fn active_notifications(&self) -> zbus::Result<Vec<String>>;

    #[zbus(name = "sendAction")]
    fn send_action(&self, key: &str, action: &str) -> zbus::Result<()>;
}

#[zbus::proxy(interface = "org.kde.kdeconnect.device.notifications.notification")]
trait KdeNotification {
    #[zbus(name = "dismiss")]
    fn dismiss(&self) -> zbus::Result<()>;

    #[zbus(name = "sendReply")]
    fn send_reply(&self, message: &str) -> zbus::Result<()>;

    #[zbus(property, name = "appName")]
    fn app_name(&self) -> zbus::Result<String>;

    #[zbus(property, name = "title")]
    fn title(&self) -> zbus::Result<String>;

    #[zbus(property, name = "text")]
    fn text(&self) -> zbus::Result<String>;

    #[zbus(property, name = "dismissable")]
    fn dismissable(&self) -> zbus::Result<bool>;

    #[zbus(property, name = "hasIcon")]
    fn has_icon(&self) -> zbus::Result<bool>;

    #[zbus(property, name = "iconPath")]
    fn icon_path(&self) -> zbus::Result<String>;

    #[zbus(property, name = "replyId")]
    fn reply_id(&self) -> zbus::Result<String>;

    #[zbus(property, name = "internalId")]
    fn internal_id(&self) -> zbus::Result<String>;
}

/// A concrete adapter for KDE Connect's session-bus service.
pub struct KdeConnectBackend {
    connection: Connection,
    devices: BTreeMap<DeviceId, Device>,
    notifications: BTreeMap<NotificationId, Notification>,
}

impl KdeConnectBackend {
    /// Connect to the desktop session bus.
    pub async fn connect() -> Result<Self, BackendError> {
        Ok(Self {
            connection: Connection::session().await?,
            devices: BTreeMap::new(),
            notifications: BTreeMap::new(),
        })
    }

    /// Watch KDE Connect and emit normalized state changes until the bus closes.
    ///
    /// The callback is synchronous, so callers can update owned state without a
    /// mutex or channel. This method does not poll device state. It re-reads a
    /// device only in response to a KDE Connect signal or service appearance.
    pub async fn run<F>(mut self, mut emit: F) -> Result<(), BackendError>
    where
        F: FnMut(StateEvent),
    {
        let dbus = DBusProxy::new(&self.connection).await?;
        let mut owner_changes = dbus
            .receive_name_owner_changed_with_args(&[(0, SERVICE)])
            .await?;
        let mut kde_signals = self.kde_signal_stream().await?;

        self.try_start_service(&dbus).await;
        if self.service_has_owner(&dbus).await? {
            info!(backend = "kdeconnect", "KDE Connect backend available");
            self.reconcile(&mut emit).await;
        } else {
            warn!(backend = "kdeconnect", "KDE Connect backend unavailable");
        }

        loop {
            tokio::select! {
                owner_change = owner_changes.next() => {
                    let Some(owner_change) = owner_change else {
                        return Err(BackendError::SignalStreamClosed("D-Bus owner"));
                    };
                    let args = owner_change.args()?;
                    if args.new_owner().is_some() {
                        if args.old_owner().is_some() {
                            self.remove_all(&mut emit);
                        }
                        info!(backend = "kdeconnect", "KDE Connect backend available");
                        self.reconcile(&mut emit).await;
                    } else {
                        warn!(backend = "kdeconnect", "KDE Connect backend unavailable");
                        self.remove_all(&mut emit);
                    }
                }
                message = kde_signals.next() => {
                    let Some(message) = message else {
                        return Err(BackendError::SignalStreamClosed("KDE Connect"));
                    };
                    match message {
                        Ok(message) => self.handle_signal(&message, &mut emit).await,
                        Err(error) => warn!(%error, "failed to receive KDE Connect signal"),
                    }
                }
            }
        }
    }

    async fn kde_signal_stream(&self) -> Result<MessageStream, BackendError> {
        let rule = MatchRule::builder()
            .msg_type(Type::Signal)
            .sender(SERVICE)?
            .path_namespace(DAEMON_PATH)?
            .build();
        Ok(MessageStream::for_match_rule(rule, &self.connection, Some(64)).await?)
    }

    async fn try_start_service(&self, dbus: &DBusProxy<'_>) {
        let service = WellKnownName::try_from(SERVICE).expect("constant service name is valid");
        if let Err(error) = dbus.start_service_by_name(service, 0).await {
            debug!(%error, "could not activate KDE Connect through D-Bus");
        }
    }

    async fn service_has_owner(&self, dbus: &DBusProxy<'_>) -> Result<bool, BackendError> {
        let service = BusName::try_from(SERVICE).expect("constant service name is valid");
        Ok(dbus.name_has_owner(service).await?)
    }

    async fn handle_signal<F>(&mut self, message: &Message, emit: &mut F)
    where
        F: FnMut(StateEvent),
    {
        let header = message.header();
        let Some(path) = header.path().map(|path| path.as_str()) else {
            return;
        };
        let Some(member) = header.member().map(|member| member.as_str()) else {
            return;
        };

        if path == DAEMON_PATH {
            match member {
                "deviceAdded" => {
                    if let Ok((id,)) = message.body().deserialize::<(String,)>() {
                        self.refresh(&id, emit).await;
                    }
                }
                "deviceVisibilityChanged" => {
                    if let Ok((id, _visible)) = message.body().deserialize::<(String, bool)>() {
                        self.refresh(&id, emit).await;
                    }
                }
                "deviceRemoved" => {
                    if let Ok((id,)) = message.body().deserialize::<(String,)>() {
                        self.remove_device(&DeviceId::new(id), emit);
                    }
                }
                "deviceListChanged" => self.reconcile(emit).await,
                _ => {}
            }
            return;
        }

        if let Some((device_id, notification_id)) = notification_id_from_path(path) {
            match member {
                "ready" | "PropertiesChanged" => {
                    self.refresh_notification(device_id, notification_id, emit)
                        .await;
                }
                _ => {}
            }
            return;
        }

        if let Some(device_id) = share_device_id_from_path(path) {
            if member == "shareReceived"
                && let Ok((resource,)) = message.body().deserialize::<(String,)>()
            {
                match normalize_received_share(device_id, &resource) {
                    Some(share) => emit(StateEvent::ShareReceived(share)),
                    None => warn!(device_id, "ignored invalid KDE Connect shareReceived value"),
                }
            }
            return;
        }

        if let Some(device_id) = notifications_device_id_from_path(path) {
            match member {
                "notificationPosted" | "notificationUpdated" => {
                    if let Ok((notification_id,)) = message.body().deserialize::<(String,)>() {
                        self.refresh_notification(device_id, &notification_id, emit)
                            .await;
                    }
                }
                "notificationRemoved" => {
                    if let Ok((notification_id,)) = message.body().deserialize::<(String,)>() {
                        self.remove_notification(
                            &NotificationId::new(DeviceId::new(device_id), notification_id),
                            emit,
                        );
                    }
                }
                "allNotificationsRemoved" => self.remove_device_notifications(device_id, emit),
                _ => {}
            }
            return;
        }

        if let Some(id) = device_id_from_path(path) {
            match member {
                "reachableChanged" | "pairStateChanged" | "nameChanged" | "pluginsChanged"
                | "refreshed" | "PropertiesChanged" => self.refresh(id, emit).await,
                _ => {}
            }
        }
    }

    async fn reconcile<F>(&mut self, emit: &mut F)
    where
        F: FnMut(StateEvent),
    {
        let daemon = match DaemonProxy::new(&self.connection).await {
            Ok(daemon) => daemon,
            Err(error) => {
                warn!(%error, "failed to create KDE Connect daemon proxy");
                return;
            }
        };
        let ids = match daemon.devices(false, false).await {
            Ok(ids) => ids,
            Err(error) => {
                warn!(%error, "failed to enumerate KDE Connect devices");
                return;
            }
        };

        let current: BTreeSet<_> = ids.iter().map(|id| DeviceId::new(id.clone())).collect();
        let removed: Vec<_> = self
            .devices
            .keys()
            .filter(|id| !current.contains(*id))
            .cloned()
            .collect();
        for id in removed {
            self.remove_device(&id, emit);
        }
        for id in ids {
            self.refresh(&id, emit).await;
        }
    }

    async fn refresh<F>(&mut self, id: &str, emit: &mut F)
    where
        F: FnMut(StateEvent),
    {
        match self.read_external_device(id).await {
            Ok(raw) => {
                let (device, battery_warning) = normalize(raw);
                if let Some(error) = battery_warning {
                    warn!(device_id = id, %error, "ignored invalid KDE Connect battery state");
                }
                let notification_available = device.connected
                    && device.paired
                    && device.capabilities.contains(&Capability::Notifications);
                self.publish_device(device, emit);
                if notification_available {
                    self.reconcile_notifications(id, emit).await;
                } else {
                    self.remove_device_notifications(id, emit);
                }
            }
            Err(error) => {
                warn!(device_id = id, %error, "failed to read KDE Connect device state");
            }
        }
    }

    async fn read_external_device(&self, id: &str) -> Result<ExternalDevice, zbus::Error> {
        let path = format!("{DEVICES_PATH}/{id}");
        let device = KdeDeviceProxy::builder(&self.connection)
            .destination(SERVICE)?
            .path(path.as_str())?
            .build()
            .await?;
        let name = device.name().await?;
        let connected = device.is_reachable().await?;
        let paired = device.is_paired().await?;
        let supported_plugins = device.supported_plugins().await?;
        let battery_supported = supported_plugins.iter().any(|name| name == BATTERY_PLUGIN);
        let notifications_supported = supported_plugins
            .iter()
            .any(|name| name == NOTIFICATIONS_PLUGIN);
        let share_supported = supported_plugins.iter().any(|name| name == SHARE_PLUGIN)
            && device
                .is_plugin_enabled(SHARE_PLUGIN)
                .await
                .unwrap_or(false);
        let battery = if connected && paired && battery_supported {
            self.read_battery(id).await
        } else {
            None
        };

        Ok(ExternalDevice {
            id: id.to_owned(),
            name,
            connected,
            paired,
            battery_supported,
            notifications_supported,
            share_supported,
            battery,
        })
    }

    async fn read_battery(&self, id: &str) -> Option<ExternalBattery> {
        let path = format!("{DEVICES_PATH}/{id}/battery");
        let battery = match BatteryProxy::builder(&self.connection)
            .destination(SERVICE)
            .and_then(|builder| builder.path(path.as_str()))
        {
            Ok(builder) => match builder.build().await {
                Ok(proxy) => proxy,
                Err(error) => {
                    debug!(device_id = id, %error, "KDE Connect battery object is unavailable");
                    return None;
                }
            },
            Err(error) => {
                debug!(device_id = id, %error, "invalid KDE Connect battery proxy address");
                return None;
            }
        };

        match battery.has_battery().await {
            Ok(false) => None,
            Ok(true) => match (battery.charge().await, battery.is_charging().await) {
                (Ok(percentage), Ok(charging)) => Some(ExternalBattery {
                    percentage,
                    charging,
                }),
                (charge, charging) => {
                    debug!(
                        device_id = id,
                        ?charge,
                        ?charging,
                        "failed to read KDE Connect battery properties"
                    );
                    None
                }
            },
            Err(error) => {
                debug!(device_id = id, %error, "KDE Connect battery object is unavailable");
                None
            }
        }
    }

    async fn reconcile_notifications<F>(&mut self, device_id: &str, emit: &mut F)
    where
        F: FnMut(StateEvent),
    {
        let notifications = match self.notifications_proxy(device_id).await {
            Ok(notifications) => notifications,
            Err(error) => {
                debug!(device_id, %error, "KDE Connect notifications object is unavailable");
                self.remove_device_notifications(device_id, emit);
                return;
            }
        };
        let ids = match notifications.active_notifications().await {
            Ok(ids) => ids,
            Err(error) => {
                debug!(device_id, %error, "failed to enumerate KDE Connect notifications");
                return;
            }
        };
        let device_id = DeviceId::new(device_id);
        let current: BTreeSet<_> = ids
            .iter()
            .map(|id| NotificationId::new(device_id.clone(), id))
            .collect();
        let removed: Vec<_> = self
            .notifications
            .keys()
            .filter(|id| id.device_id == device_id && !current.contains(*id))
            .cloned()
            .collect();
        for id in removed {
            self.remove_notification(&id, emit);
        }
        for id in ids {
            self.refresh_notification(device_id.as_str(), &id, emit)
                .await;
        }
    }

    async fn refresh_notification<F>(
        &mut self,
        device_id: &str,
        notification_id: &str,
        emit: &mut F,
    ) where
        F: FnMut(StateEvent),
    {
        match self
            .read_external_notification(device_id, notification_id)
            .await
        {
            Ok(notification) => self.publish_notification(notification, emit),
            Err(error) => debug!(
                device_id,
                notification_id,
                %error,
                "failed to read KDE Connect notification"
            ),
        }
    }

    async fn notifications_proxy(
        &self,
        device_id: &str,
    ) -> Result<NotificationsProxy<'_>, zbus::Error> {
        let path = format!("{DEVICES_PATH}/{device_id}/notifications");
        NotificationsProxy::builder(&self.connection)
            .destination(SERVICE)?
            .path(path)?
            .build()
            .await
    }

    async fn notification_proxy(
        &self,
        device_id: &str,
        notification_id: &str,
    ) -> Result<KdeNotificationProxy<'_>, zbus::Error> {
        let path = format!("{DEVICES_PATH}/{device_id}/notifications/{notification_id}");
        KdeNotificationProxy::builder(&self.connection)
            .destination(SERVICE)?
            .path(path)?
            .build()
            .await
    }

    async fn read_external_notification(
        &self,
        device_id: &str,
        notification_id: &str,
    ) -> Result<Notification, zbus::Error> {
        let notification = self.notification_proxy(device_id, notification_id).await?;
        let has_icon = notification.has_icon().await?;
        let icon_path = if has_icon {
            let path = notification.icon_path().await?;
            (!path.is_empty()).then_some(path)
        } else {
            None
        };
        Ok(normalize_notification(ExternalNotification {
            device_id: device_id.to_owned(),
            public_id: notification_id.to_owned(),
            app_name: notification.app_name().await?,
            title: notification.title().await?,
            body: notification.text().await?,
            icon_path,
            dismissable: notification.dismissable().await?,
            reply_id: notification.reply_id().await?,
        }))
    }

    fn publish_device<F>(&mut self, device: Device, emit: &mut F)
    where
        F: FnMut(StateEvent),
    {
        let event = match self.devices.get(&device.id) {
            None => DeviceEvent::Added(device.clone()),
            Some(previous) if previous != &device => DeviceEvent::Updated(device.clone()),
            Some(_) => return,
        };
        self.devices.insert(device.id.clone(), device);
        emit(StateEvent::Device(event));
    }

    fn publish_notification<F>(&mut self, notification: Notification, emit: &mut F)
    where
        F: FnMut(StateEvent),
    {
        let event = match self.notifications.get(&notification.id) {
            None => NotificationEvent::Added(notification.clone()),
            Some(previous) if previous != &notification => {
                NotificationEvent::Updated(notification.clone())
            }
            Some(_) => return,
        };
        self.notifications
            .insert(notification.id.clone(), notification);
        emit(StateEvent::Notification(event));
    }

    fn remove_device<F>(&mut self, id: &DeviceId, emit: &mut F)
    where
        F: FnMut(StateEvent),
    {
        self.remove_device_notifications(id.as_str(), emit);
        if self.devices.remove(id).is_some() {
            emit(StateEvent::Device(DeviceEvent::Removed(id.clone())));
        }
    }

    fn remove_all<F>(&mut self, emit: &mut F)
    where
        F: FnMut(StateEvent),
    {
        while let Some((id, _notification)) = self.notifications.pop_first() {
            emit(StateEvent::Notification(NotificationEvent::Removed(id)));
        }
        while let Some((id, _device)) = self.devices.pop_first() {
            emit(StateEvent::Device(DeviceEvent::Removed(id)));
        }
    }

    fn remove_device_notifications<F>(&mut self, device_id: &str, emit: &mut F)
    where
        F: FnMut(StateEvent),
    {
        let ids: Vec<_> = self
            .notifications
            .keys()
            .filter(|id| id.device_id.as_str() == device_id)
            .cloned()
            .collect();
        for id in ids {
            self.remove_notification(&id, emit);
        }
    }

    fn remove_notification<F>(&mut self, id: &NotificationId, emit: &mut F)
    where
        F: FnMut(StateEvent),
    {
        if self.notifications.remove(id).is_some() {
            emit(StateEvent::Notification(NotificationEvent::Removed(
                id.clone(),
            )));
        }
    }

    /// Execute a normalized notification command through KDE Connect.
    pub async fn execute(command: &NotificationCommand) -> Result<(), CommandError> {
        let connection = Connection::session().await?;
        let notification_id = command.notification_id();
        let path = format!(
            "{DEVICES_PATH}/{}/notifications/{}",
            notification_id.device_id, notification_id.local_id
        );
        let notification = KdeNotificationProxy::builder(&connection)
            .destination(SERVICE)?
            .path(path.as_str())?
            .build()
            .await?;

        match command {
            NotificationCommand::Dismiss { .. } => {
                if !notification.dismissable().await? {
                    return Err(CommandError::NoLongerSupported);
                }
                notification.dismiss().await?;
            }
            NotificationCommand::Reply { text, .. } => {
                if notification.reply_id().await?.is_empty() {
                    return Err(CommandError::NoLongerSupported);
                }
                notification.send_reply(text).await?;
            }
            NotificationCommand::InvokeAction { action_id, .. } => {
                let key = notification.internal_id().await?;
                let plugin_path =
                    format!("{DEVICES_PATH}/{}/notifications", notification_id.device_id);
                let plugin = NotificationsProxy::builder(&connection)
                    .destination(SERVICE)?
                    .path(plugin_path.as_str())?
                    .build()
                    .await?;
                plugin.send_action(&key, action_id).await?;
            }
        }
        Ok(())
    }

    /// Ask KDE Connect to send one URL or local-file URL to a device.
    /// A successful return means accepted by D-Bus, not transfer completion.
    pub async fn share_url(device_id: &DeviceId, url: &str) -> Result<(), CommandError> {
        let connection = Connection::session()
            .await
            .map_err(|_| CommandError::BackendUnavailable)?;
        let dbus = DBusProxy::new(&connection)
            .await
            .map_err(|_| CommandError::BackendUnavailable)?;
        let service = BusName::try_from(SERVICE).expect("constant service name is valid");
        if !dbus
            .name_has_owner(service)
            .await
            .map_err(|_| CommandError::BackendUnavailable)?
        {
            return Err(CommandError::BackendUnavailable);
        }
        let path = format!("{DEVICES_PATH}/{device_id}/share");
        let share = ShareProxy::builder(&connection)
            .destination(SERVICE)?
            .path(path.as_str())?
            .build()
            .await?;
        share.share_url(url).await?;
        Ok(())
    }
}

#[derive(Debug, Error)]
pub enum BackendError {
    #[error("D-Bus error: {0}")]
    Dbus(#[from] zbus::Error),
    #[error("D-Bus protocol error: {0}")]
    Fdo(#[from] zbus::fdo::Error),
    #[error("{0} signal stream closed")]
    SignalStreamClosed(&'static str),
}

#[derive(Debug, Error)]
pub enum CommandError {
    #[error("D-Bus error: {0}")]
    Dbus(#[from] zbus::Error),
    #[error("notification no longer supports this command")]
    NoLongerSupported,
    #[error("KDE Connect service is unavailable")]
    BackendUnavailable,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ExternalDevice {
    id: String,
    name: String,
    connected: bool,
    paired: bool,
    battery_supported: bool,
    notifications_supported: bool,
    share_supported: bool,
    battery: Option<ExternalBattery>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ExternalBattery {
    percentage: i32,
    charging: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ExternalNotification {
    device_id: String,
    public_id: String,
    app_name: String,
    title: String,
    body: String,
    icon_path: Option<String>,
    dismissable: bool,
    reply_id: String,
}

fn normalize_notification(raw: ExternalNotification) -> Notification {
    Notification {
        id: NotificationId::new(DeviceId::new(raw.device_id), raw.public_id),
        app_name: raw.app_name,
        title: raw.title,
        body: raw.body,
        icon_path: raw.icon_path,
        clearable: raw.dismissable,
        actions: Vec::new(),
        reply_supported: !raw.reply_id.is_empty(),
    }
}

fn normalize(raw: ExternalDevice) -> (Device, Option<InvalidExternalData>) {
    let (battery, warning) = match raw.battery {
        Some(raw_battery) => match u8::try_from(raw_battery.percentage)
            .ok()
            .and_then(|percentage| BatteryState::new(percentage, raw_battery.charging).ok())
        {
            Some(battery) => (Some(battery), None),
            None => (
                None,
                Some(InvalidExternalData::BatteryPercentage(
                    raw_battery.percentage,
                )),
            ),
        },
        None => (None, None),
    };
    let mut capabilities = BTreeSet::new();
    if raw.battery_supported {
        capabilities.insert(Capability::Battery);
    }
    if raw.notifications_supported {
        capabilities.insert(Capability::Notifications);
    }
    if raw.share_supported {
        capabilities.insert(Capability::FileTransfer);
    }

    (
        Device {
            id: DeviceId::new(raw.id),
            name: raw.name,
            connected: raw.connected,
            paired: raw.paired,
            battery,
            capabilities,
        },
        warning,
    )
}

fn device_id_from_path(path: &str) -> Option<&str> {
    let rest = path.strip_prefix(DEVICES_PATH)?.strip_prefix('/')?;
    rest.split('/').next().filter(|id| !id.is_empty())
}

fn notifications_device_id_from_path(path: &str) -> Option<&str> {
    let rest = path.strip_prefix(DEVICES_PATH)?.strip_prefix('/')?;
    let (device_id, suffix) = rest.split_once('/')?;
    (suffix == "notifications").then_some(device_id)
}

fn notification_id_from_path(path: &str) -> Option<(&str, &str)> {
    let rest = path.strip_prefix(DEVICES_PATH)?.strip_prefix('/')?;
    let (device_id, suffix) = rest.split_once('/')?;
    let notification_id = suffix.strip_prefix("notifications/")?;
    (!device_id.is_empty() && !notification_id.is_empty() && !notification_id.contains('/'))
        .then_some((device_id, notification_id))
}

fn share_device_id_from_path(path: &str) -> Option<&str> {
    let rest = path.strip_prefix(DEVICES_PATH)?.strip_prefix('/')?;
    let (device_id, suffix) = rest.split_once('/')?;
    (!device_id.is_empty() && suffix == "share").then_some(device_id)
}

fn normalize_received_share(device_id: &str, value: &str) -> Option<ReceivedShare> {
    let parsed = Url::parse(value).ok()?;
    let resource = if parsed.scheme() == "file" {
        let path = parsed.to_file_path().ok()?;
        SharedResource::File {
            path: path.to_str()?.to_owned(),
        }
    } else {
        SharedResource::Url { url: parsed.into() }
    };
    Some(ReceivedShare {
        device_id: DeviceId::new(device_id),
        resource,
    })
}

#[derive(Clone, Copy, Debug, Error, Eq, PartialEq)]
enum InvalidExternalData {
    #[error("battery percentage must be between 0 and 100, got {0}")]
    BatteryPercentage(i32),
}

#[cfg(test)]
mod tests {
    use super::*;

    fn external_device(battery: Option<ExternalBattery>) -> ExternalDevice {
        ExternalDevice {
            id: "phone-123".into(),
            name: "Phone".into(),
            connected: true,
            paired: true,
            battery_supported: true,
            notifications_supported: true,
            share_supported: true,
            battery,
        }
    }

    #[test]
    fn translates_external_device_state() {
        let (device, warning) = normalize(external_device(None));

        assert_eq!(device.id, DeviceId::new("phone-123"));
        assert_eq!(device.name, "Phone");
        assert!(device.connected);
        assert!(device.paired);
        assert_eq!(
            device.capabilities,
            BTreeSet::from([
                Capability::Battery,
                Capability::Notifications,
                Capability::FileTransfer,
            ])
        );
        assert_eq!(warning, None);
    }

    #[test]
    fn translates_valid_battery_state() {
        let (device, warning) = normalize(external_device(Some(ExternalBattery {
            percentage: 83,
            charging: false,
        })));

        assert_eq!(
            device.battery,
            Some(BatteryState::new(83, false).expect("valid battery"))
        );
        assert_eq!(warning, None);
    }

    #[test]
    fn rejects_invalid_battery_state_without_rejecting_device() {
        for percentage in [-1, 101, 300] {
            let (device, warning) = normalize(external_device(Some(ExternalBattery {
                percentage,
                charging: true,
            })));

            assert_eq!(device.battery, None);
            assert_eq!(
                warning,
                Some(InvalidExternalData::BatteryPercentage(percentage))
            );
        }
    }

    #[test]
    fn extracts_device_id_from_device_and_plugin_paths() {
        assert_eq!(
            device_id_from_path("/modules/kdeconnect/devices/phone-123"),
            Some("phone-123")
        );
        assert_eq!(
            device_id_from_path("/modules/kdeconnect/devices/phone-123/battery"),
            Some("phone-123")
        );
        assert_eq!(device_id_from_path("/modules/kdeconnect"), None);
    }

    #[test]
    fn extracts_notification_paths_without_exposing_them() {
        assert_eq!(
            notifications_device_id_from_path(
                "/modules/kdeconnect/devices/phone-123/notifications"
            ),
            Some("phone-123")
        );
        assert_eq!(
            notification_id_from_path("/modules/kdeconnect/devices/phone-123/notifications/42"),
            Some(("phone-123", "42"))
        );
        assert_eq!(
            notification_id_from_path(
                "/modules/kdeconnect/devices/phone-123/notifications/42/extra"
            ),
            None
        );
        assert_eq!(
            notification_id_from_path("/modules/kdeconnect/devices/phone-123/notifications/"),
            None
        );
    }

    #[test]
    fn translates_notification_without_leaking_reply_token() {
        let notification = normalize_notification(ExternalNotification {
            device_id: "phone-123".into(),
            public_id: "42".into(),
            app_name: "Messages".into(),
            title: "Alice".into(),
            body: "Hello".into(),
            icon_path: None,
            dismissable: true,
            reply_id: "private-token".into(),
        });

        assert_eq!(
            notification.id,
            NotificationId::new(DeviceId::new("phone-123"), "42")
        );
        assert!(notification.clearable);
        assert!(notification.reply_supported);
        assert!(notification.actions.is_empty());
        assert!(!format!("{notification:?}").contains("private-token"));
    }

    #[test]
    fn missing_reply_token_is_not_advertised() {
        let notification = normalize_notification(ExternalNotification {
            device_id: "phone-123".into(),
            public_id: "42".into(),
            app_name: "Calendar".into(),
            title: "Meeting".into(),
            body: String::new(),
            icon_path: None,
            dismissable: false,
            reply_id: String::new(),
        });

        assert!(!notification.clearable);
        assert!(!notification.reply_supported);
    }

    #[test]
    fn translates_received_file_and_url_with_source_device() {
        let file = normalize_received_share("phone-a", "file:///tmp/handover%20%E2%9C%93.txt")
            .expect("valid local file URL");
        assert_eq!(file.device_id, DeviceId::new("phone-a"));
        assert_eq!(
            file.resource,
            SharedResource::File {
                path: "/tmp/handover ✓.txt".into()
            }
        );

        let url = normalize_received_share("phone-b", "https://example.com/a?q=1")
            .expect("valid remote URL");
        assert_eq!(url.device_id, DeviceId::new("phone-b"));
        assert_eq!(
            url.resource,
            SharedResource::Url {
                url: "https://example.com/a?q=1".into()
            }
        );
    }

    #[test]
    fn rejects_malformed_received_share() {
        assert!(normalize_received_share("phone-a", "not a URL").is_none());
        assert!(normalize_received_share("phone-a", "file://other-host/tmp/a").is_none());
        assert_eq!(
            share_device_id_from_path("/modules/kdeconnect/devices/phone-a/share"),
            Some("phone-a")
        );
    }
}
