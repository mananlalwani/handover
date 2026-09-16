use std::process::ExitCode;
use std::time::Duration;

use clap::{CommandFactory, Parser, Subcommand};
use handover_core::{Device, Notification};
use handover_ipc::{Client, IpcError, PROTOCOL_VERSION, ServerPayload};

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
    /// List active remote notifications
    Notifications,
    /// Print live normalized device and notification changes
    Monitor,
}

#[tokio::main]
async fn main() -> ExitCode {
    let cli = Cli::parse();

    let result = match cli.command {
        Some(Command::Devices) => list_devices().await,
        Some(Command::Notifications) => list_notifications().await,
        Some(Command::Monitor) => monitor().await,
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

async fn list_devices() -> Result<(), IpcError> {
    let mut client = connected_client().await?;
    let devices = client.devices().await?;
    print_table(&devices);
    Ok(())
}

async fn list_notifications() -> Result<(), IpcError> {
    let mut client = connected_client().await?;
    let devices = client.devices().await?;
    let notifications = client.notifications().await?;
    print_notification_table(&devices, &notifications);
    Ok(())
}

async fn monitor() -> Result<(), IpcError> {
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
        ServerPayload::Snapshot {
            devices,
            notifications,
        } => {
            println!(
                "state resynchronized: {} device(s), {} notification(s)",
                devices.len(),
                notifications.len()
            );
        }
        ServerPayload::Error { code, message } => {
            eprintln!("daemon error ({code:?}): {message}");
        }
        ServerPayload::Hello { .. }
        | ServerPayload::Devices { .. }
        | ServerPayload::Notifications { .. }
        | ServerPayload::CommandCompleted { .. }
        | ServerPayload::Subscribed { .. } => {}
    }
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

    use handover_core::{BatteryState, Capability, DeviceId};

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
}
