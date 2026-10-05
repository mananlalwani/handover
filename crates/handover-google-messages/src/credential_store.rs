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
