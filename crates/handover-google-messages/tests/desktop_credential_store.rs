//! Real GNOME Keyring interoperability. The launcher supplies a private bus and storage.

use handover_google_messages::{
    BrowserProof,
    credential_store::{CredentialError, DesktopCredentialStore, watch_unlocks},
};
use secret_service::{EncryptionType, SecretService};
use std::{collections::HashMap, path::PathBuf, process::Stdio, time::Duration};
use tokio::{io::AsyncWriteExt, process::Child, sync::mpsc};

fn proof() -> BrowserProof {
    BrowserProof {
        kind: "gaia_pairing".into(),
        endpoint: "https://instantmessaging-pa.googleapis.com".into(),
        origin: "https://messages.google.com".into(),
        authorization: "Bearer SYNTHETIC_AUTH".into(),
        api_key: "SYNTHETIC_KEY".into(),
        auth_user: Some("0".into()),
        service_cookie: Some("SID=SYNTHETIC_COOKIE".into()),
        account_email: Some("fixture@example.test".into()),
        browser_request: None,
    }
}

fn isolated_root() -> PathBuf {
    let root = PathBuf::from(
        std::env::var_os("HANDOVER_SECRET_SERVICE_TEST_ROOT")
            .expect("run scripts/test-secret-service.py; never use the desktop bus"),
    );
    for name in [
        "DATA_HOME",
        "CONFIG_HOME",
        "STATE_HOME",
        "CACHE_HOME",
        "RUNTIME_DIR",
    ] {
        assert_eq!(
            PathBuf::from(std::env::var_os(format!("XDG_{name}")).unwrap()),
            root.join(name.to_lowercase()),
            "test storage must be private"
        );
    }
    assert!(
        std::env::var("DBUS_SESSION_BUS_ADDRESS")
            .unwrap()
            .starts_with(&format!("unix:path={}/runtime_dir/bus", root.display()))
    );
    assert!(root.join("data_home/keyrings").is_dir());
    root
}

async fn start_keyring(root: &std::path::Path, password: Option<&str>) -> Child {
    let mut command = tokio::process::Command::new("gnome-keyring-daemon");
    command
        .args([
            "--foreground",
            "--components=secrets",
            "--control-directory",
        ])
        .arg(root.join("runtime_dir/keyring"))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    if password.is_some() {
        command.arg("--unlock");
    }
    let mut child = command.spawn().unwrap();
    let mut input = child.stdin.take().unwrap();
    if let Some(password) = password {
        input.write_all(password.as_bytes()).await.unwrap();
    }
    drop(input);
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            assert!(
                child.try_wait().unwrap().is_none(),
                "private keyring exited"
            );
            if let Ok(service) = SecretService::connect(EncryptionType::Dh).await
                && let Ok(collection) = service.get_default_collection().await
                && collection.is_locked().await.unwrap() == password.is_none()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    })
    .await
    .expect("private keyring readiness");
    child
}

async fn unlock_keyring(password: &str) {
    // Use GNOME's password-unlock extension only in this private test, without a GUI.
    // Production storage continues to use secret-service's encrypted DH sessions.
    let connection = zbus::Connection::session().await.unwrap();
    let service = zbus::Proxy::new(
        &connection,
        "org.freedesktop.secrets",
        "/org/freedesktop/secrets",
        "org.freedesktop.Secret.Service",
    )
    .await
    .unwrap();
    let (_, session): (zbus::zvariant::OwnedValue, zbus::zvariant::OwnedObjectPath) = service
        .call("OpenSession", &("plain", zbus::zvariant::Value::from("")))
        .await
        .unwrap();
    let collection: zbus::zvariant::OwnedObjectPath =
        service.call("ReadAlias", &("default",)).await.unwrap();
    let internal = zbus::Proxy::new(
        &connection,
        "org.freedesktop.secrets",
        "/org/freedesktop/secrets",
        "org.gnome.keyring.InternalUnsupportedGuiltRiddenInterface",
    )
    .await
    .unwrap();
    let secret = (session, Vec::<u8>::new(), password.as_bytes(), "text/plain");
    let _: () = internal
        .call("UnlockWithMasterPassword", &(collection, secret))
        .await
        .unwrap();
}

async fn assert_blocked(account: &str, error: CredentialError) {
    tokio::time::timeout(Duration::from_secs(2), async {
        assert_eq!(DesktopCredentialStore::check_writable().await, Err(error));
        assert_eq!(
            DesktopCredentialStore::save(account, &proof()).await,
            Err(error)
        );
        assert_eq!(
            DesktopCredentialStore::load(account)
                .await
                .map(|p| p.is_some()),
            Err(error)
        );
        assert_eq!(DesktopCredentialStore::delete(account).await, Err(error));
    })
    .await
    .expect("store operations must fail without an unlock prompt");
}

async fn assert_loaded(account: &str, expected: &BrowserProof) {
    let loaded = DesktopCredentialStore::load(account)
        .await
        .unwrap()
        .unwrap();
    assert!(
        serde_json::to_value(&loaded).unwrap() == serde_json::to_value(expected).unwrap(),
        "stored proof changed"
    );
}

async fn quiet(notifications: &mut mpsc::Receiver<()>) {
    loop {
        match tokio::time::timeout(Duration::from_millis(150), notifications.recv()).await {
            Ok(Some(())) => {}
            Ok(None) => panic!("credential watcher stopped"),
            Err(_) => break,
        }
    }
}

#[tokio::test]
#[ignore = "run scripts/test-secret-service.py on a private bus"]
async fn desktop_store_lifecycle() {
    let root = isolated_root();
    let account = format!("gmessages-{}", uuid::Uuid::new_v4().simple());
    let password = uuid::Uuid::new_v4().to_string();
    assert_blocked(&account, CredentialError::Unavailable).await;
    let mut keyring = start_keyring(&root, Some(&password)).await;
    assert_eq!(DesktopCredentialStore::check_writable().await, Ok(()));
    assert!(
        DesktopCredentialStore::load(&account)
            .await
            .unwrap()
            .is_none()
    );
    DesktopCredentialStore::save(&account, &proof())
        .await
        .unwrap();
    let mut updated = proof();
    updated.authorization = "Bearer SYNTHETIC_UPDATED_AUTH".into();
    DesktopCredentialStore::save(&account, &updated)
        .await
        .unwrap();
    assert_loaded(&account, &updated).await;

    let service = SecretService::connect(EncryptionType::Dh).await.unwrap();
    let collection = service.get_default_collection().await.unwrap();
    let attributes = HashMap::from([
        ("application", "handover-google-messages"),
        ("account", account.as_str()),
        ("purpose", "google-auth-v1"),
    ]);
    assert_eq!(
        service
            .search_items(attributes.clone())
            .await
            .unwrap()
            .unlocked
            .len(),
        1
    );

    let (notify, mut notifications) = mpsc::channel(1);
    let watcher = tokio::spawn(watch_unlocks(notify));
    // Establish readiness through real unlock events, without a production test hook.
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            collection.lock().await.unwrap();
            unlock_keyring(&password).await;
            assert!(!collection.is_locked().await.unwrap());
            if matches!(
                tokio::time::timeout(Duration::from_millis(100), notifications.recv()).await,
                Ok(Some(()))
            ) {
                break;
            }
        }
    })
    .await
    .expect("watcher must receive real keyring change notifications");
    quiet(&mut notifications).await;
    collection.lock().await.unwrap();
    assert!(collection.is_locked().await.unwrap());
    assert_blocked(&account, CredentialError::Locked).await;
    assert!(
        tokio::time::timeout(Duration::from_millis(200), notifications.recv())
            .await
            .is_err(),
        "locking a collection must not wake credential recovery"
    );
    unlock_keyring(&password).await;
    tokio::time::timeout(Duration::from_secs(5), notifications.recv())
        .await
        .unwrap()
        .unwrap();
    assert_loaded(&account, &updated).await;
    quiet(&mut notifications).await;

    keyring.kill().await.unwrap();
    keyring.wait().await.unwrap();
    assert_blocked(&account, CredentialError::Unavailable).await;
    keyring = start_keyring(&root, None).await;
    assert_blocked(&account, CredentialError::Locked).await;
    quiet(&mut notifications).await;
    unlock_keyring(&password).await;
    tokio::time::timeout(Duration::from_secs(5), notifications.recv())
        .await
        .unwrap()
        .unwrap();
    assert_loaded(&account, &updated).await;

    // Independent Secret Service writes exercise rejection of malformed and duplicate records.
    let service = SecretService::connect(EncryptionType::Dh).await.unwrap();
    let collection = service.get_default_collection().await.unwrap();
    let items = service.search_items(attributes.clone()).await.unwrap();
    items.unlocked[0]
        .set_secret(b"{}", "application/octet-stream")
        .await
        .unwrap();
    assert!(matches!(
        DesktopCredentialStore::load(&account).await,
        Err(CredentialError::Invalid)
    ));
    items.unlocked[0]
        .set_secret(b"{}", "text/plain")
        .await
        .unwrap();
    assert!(matches!(
        DesktopCredentialStore::load(&account).await,
        Err(CredentialError::Invalid)
    ));
    DesktopCredentialStore::save(&account, &updated)
        .await
        .unwrap();
    assert_loaded(&account, &updated).await;
    collection
        .create_item(
            "Synthetic duplicate",
            attributes,
            b"{}",
            false,
            "text/plain",
        )
        .await
        .unwrap();
    assert!(matches!(
        DesktopCredentialStore::load(&account).await,
        Err(CredentialError::Ambiguous)
    ));
    assert_eq!(
        DesktopCredentialStore::save(&account, &updated).await,
        Err(CredentialError::Ambiguous)
    );
    DesktopCredentialStore::delete(&account).await.unwrap();
    DesktopCredentialStore::delete(&account).await.unwrap();
    assert!(
        DesktopCredentialStore::load(&account)
            .await
            .unwrap()
            .is_none()
    );
    watcher.abort();
    keyring.kill().await.unwrap();
    keyring.wait().await.unwrap();
}
