use futures_util::StreamExt;
use std::error::Error;
use tokio::signal::unix::{SignalKind, signal};
use tokio::sync::watch;

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    tracing_subscriber::fmt::init();
    let connection = zbus::connection::Builder::session()?
        .name("org.handover.Desktop1")?
        .build()
        .await?;
    let (shutdown, receiver) = watch::channel(false);
    tokio::spawn(async move {
        let mut terminate = signal(SignalKind::terminate()).expect("install SIGTERM handler");
        tokio::select! {
            _ = tokio::signal::ctrl_c() => {}
            _ = terminate.recv() => {}
        }
        let _ = shutdown.send(true);
    });
    let path = handover_ipc::socket_path()?;
    let address = zbus::Address::session()?.to_string();
    let mut bus_messages = zbus::MessageStream::from(&connection);
    tokio::select! {
        _ = async {
            tokio::join!(
                handover_desktop::run(path.clone(), connection, receiver.clone()),
                handover_desktop::run_media(path, address, receiver),
            );
        } => {}
        _ = async { while let Some(Ok(_)) = bus_messages.next().await {} } => {
            return Err("desktop session bus disconnected".into());
        }
    }
    Ok(())
}
