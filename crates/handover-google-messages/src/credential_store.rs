//! Google authentication stored only in the desktop Secret Service.
use crate::BrowserProof;
use secret_service::{EncryptionType, SecretService};
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, time::Duration};
use zeroize::{Zeroize, Zeroizing};

const LIMIT: usize = 48 * 1024;
const TIMEOUT: Duration = Duration::from_secs(10);
const CONTENT_TYPE: &str = "text/plain";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CredentialError {
    Missing,
    Invalid,
    Unavailable,
    Locked,
    Ambiguous,
    Timeout,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u8,
    account: String,
    proof: BrowserProof,
}
impl Drop for Record {
    fn drop(&mut self) {
        self.account.zeroize();
    }
}

fn attributes(account: &str) -> Result<HashMap<&str, &str>, CredentialError> {
    handover_core::messaging::check_account_id(account).map_err(|_| CredentialError::Invalid)?;
    if !account.strip_prefix("gmessages-").is_some_and(|id| {
        id.len() == 32
            && id
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }) {
        return Err(CredentialError::Invalid);
    }
    Ok(HashMap::from([
        ("application", "handover-google-messages"),
        ("account", account),
        ("purpose", "google-auth-v1"),
    ]))
}

fn validate(proof: &BrowserProof) -> Result<(), CredentialError> {
    proof
        .validate_pairing()
        .map_err(|_| CredentialError::Invalid)?;
    if proof.kind != "gaia_pairing" || proof.browser_request.is_some() {
        return Err(CredentialError::Invalid);
    }
    Ok(())
}

fn encode(account: &str, proof: &BrowserProof) -> Result<Zeroizing<Vec<u8>>, CredentialError> {
    attributes(account)?;
    validate(proof)?;
    #[derive(Serialize)]
    struct Borrowed<'a> {
        version: u8,
        account: &'a str,
        proof: &'a BrowserProof,
    }
    let bytes = Zeroizing::new(
        serde_json::to_vec(&Borrowed {
            version: 1,
            account,
            proof,
        })
        .map_err(|_| CredentialError::Invalid)?,
    );
    if bytes.len() > LIMIT {
        return Err(CredentialError::Invalid);
    }
    Ok(bytes)
}

fn decode(account: &str, bytes: &[u8]) -> Result<BrowserProof, CredentialError> {
    attributes(account)?;
    if bytes.len() > LIMIT {
        return Err(CredentialError::Invalid);
    }
    let mut record: Record = serde_json::from_slice(bytes).map_err(|_| CredentialError::Invalid)?;
    if record.version != 1 || record.account != account {
        return Err(CredentialError::Invalid);
    }
    validate(&record.proof)?;
    // Move the proof without copying credential strings. Record has Drop.
    let proof = std::mem::replace(
        &mut record.proof,
        BrowserProof {
            kind: String::new(),
            endpoint: String::new(),
            origin: String::new(),
            authorization: String::new(),
            api_key: String::new(),
            auth_user: None,
            service_cookie: None,
            account_email: None,
            browser_request: None,
        },
    );
    Ok(proof)
}

/// No file fallback, automatic unlock prompt, or connection-state claim.
pub struct DesktopCredentialStore;
impl DesktopCredentialStore {
    /// Check the approved store before registration or phone pairing changes state.
    pub async fn check_writable() -> Result<(), CredentialError> {
        tokio::time::timeout(TIMEOUT, async {
            let service = SecretService::connect(EncryptionType::Dh)
                .await
                .map_err(|_| CredentialError::Unavailable)?;
            let collection = service
                .get_default_collection()
                .await
                .map_err(|_| CredentialError::Unavailable)?;
            if collection
                .is_locked()
                .await
                .map_err(|_| CredentialError::Unavailable)?
            {
                return Err(CredentialError::Locked);
            }
            Ok(())
        })
        .await
        .map_err(|_| CredentialError::Timeout)?
    }
    pub async fn save(account: &str, proof: &BrowserProof) -> Result<(), CredentialError> {
        let bytes = encode(account, proof)?;
        tokio::time::timeout(TIMEOUT, async {
            let service = SecretService::connect(EncryptionType::Dh)
                .await
                .map_err(|_| CredentialError::Unavailable)?;
            let collection = service
                .get_default_collection()
                .await
                .map_err(|_| CredentialError::Unavailable)?;
            if collection
                .is_locked()
                .await
                .map_err(|_| CredentialError::Unavailable)?
            {
                return Err(CredentialError::Locked);
            }
            let mut items = service
                .search_items(attributes(account)?)
                .await
                .map_err(|_| CredentialError::Unavailable)?;
            if !items.locked.is_empty() {
                return Err(CredentialError::Locked);
            }
            if items.unlocked.len() > 1 {
                return Err(CredentialError::Ambiguous);
            }
            if let Some(item) = items.unlocked.pop() {
                item.set_secret(&bytes, CONTENT_TYPE)
                    .await
                    .map_err(|_| CredentialError::Unavailable)?;
            } else {
                collection
                    .create_item(
                        "Handover Google Messages authentication",
                        attributes(account)?,
                        &bytes,
                        false,
                        CONTENT_TYPE,
                    )
                    .await
                    .map_err(|_| CredentialError::Unavailable)?;
            }
            Ok(())
        })
        .await
        .map_err(|_| CredentialError::Timeout)?
    }

    pub async fn load(account: &str) -> Result<Option<BrowserProof>, CredentialError> {
        let attrs = attributes(account)?;
        tokio::time::timeout(TIMEOUT, async {
            let service = SecretService::connect(EncryptionType::Dh)
                .await
                .map_err(|_| CredentialError::Unavailable)?;
            let mut items = service
                .search_items(attrs)
                .await
                .map_err(|_| CredentialError::Unavailable)?;
            if !items.locked.is_empty() {
                return Err(CredentialError::Locked);
            }
            if items.unlocked.len() > 1 {
                return Err(CredentialError::Ambiguous);
            }
            let Some(item) = items.unlocked.pop() else {
                return Ok(None);
            };
            let content_type = item
                .get_secret_content_type()
                .await
                .map_err(|_| CredentialError::Unavailable)?;
            if content_type != CONTENT_TYPE {
                return Err(CredentialError::Invalid);
            }
            let bytes = Zeroizing::new(
                item.get_secret()
                    .await
                    .map_err(|_| CredentialError::Unavailable)?,
            );
            decode(account, &bytes).map(Some)
        })
        .await
        .map_err(|_| CredentialError::Timeout)?
    }

    pub async fn delete(account: &str) -> Result<(), CredentialError> {
        let attrs = attributes(account)?;
        tokio::time::timeout(TIMEOUT, async {
            let service = SecretService::connect(EncryptionType::Dh)
                .await
                .map_err(|_| CredentialError::Unavailable)?;
            let items = service
                .search_items(attrs)
                .await
                .map_err(|_| CredentialError::Unavailable)?;
            if !items.locked.is_empty() {
                return Err(CredentialError::Locked);
            }
            for item in items.unlocked {
                item.delete()
                    .await
                    .map_err(|_| CredentialError::Unavailable)?;
            }
            Ok(())
        })
        .await
        .map_err(|_| CredentialError::Timeout)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const ACCOUNT: &str = "gmessages-0123456789abcdef0123456789abcdef";
    fn proof() -> BrowserProof {
        BrowserProof {
            kind: "gaia_pairing".into(),
            endpoint: crate::ENDPOINTS[0].into(),
            origin: crate::ORIGIN_VALUE.into(),
            authorization: "Bearer SYNTHETIC_AUTH".into(),
            api_key: "SYNTHETIC_KEY".into(),
            auth_user: Some("0".into()),
            service_cookie: Some("SID=SYNTHETIC_COOKIE".into()),
            account_email: Some("fixture@example.test".into()),
            browser_request: None,
        }
    }
    #[test]
    fn credentials_are_bound_validated_and_redacted() {
        let bytes = encode(ACCOUNT, &proof()).unwrap();
        let restored = decode(ACCOUNT, &bytes).unwrap();
        assert_eq!(
            restored.service_cookie.as_deref(),
            Some("SID=SYNTHETIC_COOKIE")
        );
        assert_eq!(format!("{restored:?}"), "BrowserProof { redacted }");
        assert!(decode("gmessages-ffffffffffffffffffffffffffffffff", &bytes).is_err());
        assert!(decode(ACCOUNT, &vec![0; LIMIT + 1]).is_err());
        let mut invalid = proof();
        invalid.service_cookie = None;
        assert!(encode(ACCOUNT, &invalid).is_err());
        assert!(encode("fixture@example.test", &proof()).is_err());
    }
    #[tokio::test]
    #[ignore = "requires an unlocked desktop Secret Service"]
    async fn desktop_store_roundtrip_and_delete() {
        let account = format!("gmessages-{}", uuid::Uuid::new_v4().simple());
        let result = async {
            DesktopCredentialStore::save(&account, &proof()).await?;
            DesktopCredentialStore::save(&account, &proof()).await?;
            let loaded = DesktopCredentialStore::load(&account)
                .await?
                .ok_or(CredentialError::Invalid)?;
            if loaded.service_cookie.as_deref() != Some("SID=SYNTHETIC_COOKIE") {
                return Err(CredentialError::Invalid);
            }
            Ok(())
        }
        .await;
        let cleanup = DesktopCredentialStore::delete(&account).await;
        assert_eq!(cleanup, Ok(()));
        assert_eq!(result, Ok(()));
        assert!(
            DesktopCredentialStore::load(&account)
                .await
                .unwrap()
                .is_none()
        );
    }
}

/// Wake recovery on Secret Service unlock notifications. The bounded channel
/// coalesces notifications; retry timers remain the fallback if D-Bus is absent.
pub async fn watch_unlocks(notify: tokio::sync::mpsc::Sender<()>) {
    use futures_util::StreamExt;
    use zbus::Connection;
    loop {
        let watch = async {
            let connection = Connection::session().await?;
            let mut stream = unlock_stream(&connection).await?;
            while let Some(message) = stream.next().await {
                let message = message?;
                if unlock_notification(&message, &connection).await {
                    let _ = notify.try_send(());
                }
            }
            Ok::<_, zbus::Error>(())
        };
        let _ = watch.await;
        if notify.is_closed() {
            return;
        }
        tokio::time::sleep(Duration::from_secs(60)).await;
    }
}

async fn unlock_notification(message: &zbus::Message, connection: &zbus::Connection) -> bool {
    let header = message.header();
    let interface = header.interface().map(|value| value.as_str());
    let member = header.member().map(|value| value.as_str());
    let (path, target) = match (interface, member) {
        (Some("org.freedesktop.DBus.Properties"), Some("PropertiesChanged")) => {
            let Ok((target, changed, invalidated)) = message.body().deserialize::<(
                String,
                HashMap<String, zbus::zvariant::OwnedValue>,
                Vec<String>,
            )>() else {
                return false;
            };
            if !matches!(
                target.as_str(),
                "org.freedesktop.Secret.Collection" | "org.freedesktop.Secret.Item"
            ) {
                return false;
            }
            if let Some(locked) = changed.get("Locked") {
                return bool::try_from(locked).is_ok_and(|locked| !locked);
            }
            if !invalidated.iter().any(|property| property == "Locked") {
                return false;
            }
            let Some(path) = header.path() else {
                return false;
            };
            (path.to_owned(), target)
        }
        (Some("org.freedesktop.Secret.Service"), Some("CollectionChanged")) => {
            let Ok(path) = message
                .body()
                .deserialize::<zbus::zvariant::OwnedObjectPath>()
            else {
                return false;
            };
            (path.into(), "org.freedesktop.Secret.Collection".into())
        }
        (Some("org.freedesktop.Secret.Collection"), Some("ItemChanged")) => {
            let Ok(path) = message
                .body()
                .deserialize::<zbus::zvariant::OwnedObjectPath>()
            else {
                return false;
            };
            (path.into(), "org.freedesktop.Secret.Item".into())
        }
        _ => return false,
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        let proxy = zbus::Proxy::new(connection, "org.freedesktop.secrets", path, target).await?;
        proxy.get_property::<bool>("Locked").await
    })
    .await
    .is_ok_and(|result| result.is_ok_and(|locked| !locked))
}

async fn unlock_stream(connection: &zbus::Connection) -> Result<zbus::MessageStream, zbus::Error> {
    let rule = zbus::MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender("org.freedesktop.secrets")?
        .path_namespace("/org/freedesktop/secrets")?
        .build();
    zbus::MessageStream::for_match_rule(rule, connection, Some(16)).await
}

#[cfg(test)]
mod unlock_tests {
    use super::*;
    use futures_util::StreamExt;
    use tokio::io::{AsyncBufReadExt, BufReader};

    struct TestCollection(std::sync::Arc<std::sync::atomic::AtomicBool>);

    #[zbus::interface(name = "org.freedesktop.Secret.Collection")]
    impl TestCollection {
        #[zbus(property)]
        fn locked(&self) -> bool {
            self.0.load(std::sync::atomic::Ordering::Relaxed)
        }
    }

    #[tokio::test]
    async fn secret_service_unlock_signals_wake_recovery_but_lock_signals_do_not() {
        let mut bus = tokio::process::Command::new("dbus-daemon")
            .args(["--session", "--nofork", "--print-address=1"])
            .stdout(std::process::Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .expect("private test D-Bus");
        let mut address = String::new();
        BufReader::new(bus.stdout.take().unwrap())
            .read_line(&mut address)
            .await
            .unwrap();
        let subscriber = zbus::connection::Builder::address(address.trim())
            .unwrap()
            .build()
            .await
            .unwrap();
        let service = zbus::connection::Builder::address(address.trim())
            .unwrap()
            .build()
            .await
            .unwrap();
        service
            .request_name("org.freedesktop.secrets")
            .await
            .unwrap();
        let locked_state = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(true));
        service
            .object_server()
            .at(
                "/org/freedesktop/secrets/collection/test",
                TestCollection(locked_state.clone()),
            )
            .await
            .unwrap();
        let mut stream = unlock_stream(&subscriber).await.unwrap();
        for target in [
            "org.freedesktop.Secret.Collection",
            "org.freedesktop.Secret.Item",
            "unrelated.Interface",
        ] {
            for locked in [true, false] {
                let changed = HashMap::from([("Locked", zbus::zvariant::Value::from(locked))]);
                service
                    .emit_signal(
                        None::<&str>,
                        "/org/freedesktop/secrets/collection/test",
                        "org.freedesktop.DBus.Properties",
                        "PropertiesChanged",
                        &(target, changed, Vec::<String>::new()),
                    )
                    .await
                    .unwrap();
                let message = tokio::time::timeout(Duration::from_secs(2), stream.next())
                    .await
                    .unwrap()
                    .unwrap()
                    .unwrap();
                assert_eq!(
                    unlock_notification(&message, &subscriber).await,
                    !locked && target != "unrelated.Interface"
                );
            }
        }
        for locked in [true, false] {
            locked_state.store(locked, std::sync::atomic::Ordering::Relaxed);
            service
                .emit_signal(
                    None::<&str>,
                    "/org/freedesktop/secrets",
                    "org.freedesktop.Secret.Service",
                    "CollectionChanged",
                    &zbus::zvariant::ObjectPath::try_from(
                        "/org/freedesktop/secrets/collection/test",
                    )
                    .unwrap(),
                )
                .await
                .unwrap();
            let message = tokio::time::timeout(Duration::from_secs(2), stream.next())
                .await
                .unwrap()
                .unwrap()
                .unwrap();
            assert_eq!(unlock_notification(&message, &subscriber).await, !locked);
        }
        bus.kill().await.unwrap();
    }
}
