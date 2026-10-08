//! Desktop lifecycle signals, kept below the normalized helper contract.
use futures_util::StreamExt;
use tokio::sync::mpsc;

async fn stream(connection: &zbus::Connection) -> Result<zbus::MessageStream, zbus::Error> {
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.login1")?
        .interface("org.freedesktop.login1.Manager")?
        .path("/org/freedesktop/login1")?
        .member("PrepareForSleep")?
        .build();
    zbus::MessageStream::for_match_rule(rule, connection, Some(8)).await
}

fn resumed(message: &zbus::Message) -> bool {
    message
        .body()
        .deserialize::<bool>()
        .is_ok_and(|sleeping| !sleeping)
}

pub(super) async fn watch(notify: mpsc::Sender<()>) {
    loop {
        let watch = async {
            let connection = zbus::Connection::system().await?;
            let mut messages = stream(&connection).await?;
            while let Some(message) = messages.next().await {
                if resumed(&message?) {
                    let _ = notify.try_send(());
                }
                if notify.is_closed() {
                    break;
                }
            }
            Ok::<_, zbus::Error>(())
        };
        let _ = watch.await;
        if notify.is_closed() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::{AsyncBufReadExt, BufReader};

    #[tokio::test]
    async fn only_resume_signals_from_login1_trigger_recovery() {
        let mut bus = tokio::process::Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .unwrap();
        let mut address = String::new();
        BufReader::new(bus.stdout.take().unwrap())
            .read_line(&mut address)
            .await
            .unwrap();
        let sender = zbus::connection::Builder::address(address.trim())
            .unwrap()
            .name("org.freedesktop.login1")
            .unwrap()
            .build()
            .await
            .unwrap();
        let client = zbus::connection::Builder::address(address.trim())
            .unwrap()
            .build()
            .await
            .unwrap();
        let mut messages = stream(&client).await.unwrap();
        // A different bus owner cannot trigger lifecycle recovery.
        client
            .emit_signal(
                None::<&str>,
                "/org/freedesktop/login1",
                "org.freedesktop.login1.Manager",
                "PrepareForSleep",
                &false,
            )
            .await
            .unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(30), messages.next())
                .await
                .is_err()
        );
        for sleeping in [true, false] {
            sender
                .emit_signal(
                    None::<&str>,
                    "/org/freedesktop/login1",
                    "org.freedesktop.login1.Manager",
                    "PrepareForSleep",
                    &sleeping,
                )
                .await
                .unwrap();
            let message = tokio::time::timeout(std::time::Duration::from_secs(2), messages.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert_eq!(resumed(&message), !sleeping);
        }
    }
}
