//! Helper-owned, bounded phone pairing. Browser proof is never persisted.

use std::{fmt, time::Duration};

use base64::{Engine, engine::general_purpose};
use handover_core::messaging::check_account_id;
use handover_gmessages::contract::MAX_BUNDLE_BYTES;
use prost::Message;
use tokio::sync::{mpsc, oneshot};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

use crate::{
    BrowserProof, ProbeError,
    pairing::gaia::PhoneConfirmedPairing,
    receive::{AckBatch, Acknowledgement, PairingReply, ReceiveError, ReceiveEvent},
    registration::UnpairedRegistration,
    session_store::SessionStore,
};

const LOGIN_MAGIC: &[u8] = b"HOVL\x01\0";
const MAX_PAIRING_TIME: Duration = Duration::from_secs(300);
const RECEIVE_QUEUE: usize = 8;

pub struct LoginBootstrap {
    proof: BrowserProof,
    registration: UnpairedRegistration,
    start_pairing: bool,
    store: SessionStore,
}

impl fmt::Debug for LoginBootstrap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("LoginBootstrap { redacted }")
    }
}

/// Retained privately by the helper. Phone confirmation alone does not establish
/// a usable messaging session, so this must not become an online account.
pub struct ConfirmedPairing {
    pub(crate) registration: UnpairedRegistration,
    pub(crate) pairing: PhoneConfirmedPairing,
}

impl fmt::Debug for ConfirmedPairing {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ConfirmedPairing { redacted }")
    }
}

impl ConfirmedPairing {
    fn persist(&self, store: &SessionStore) -> Result<(), ProbeError> {
        let record = StoredConfirmedPairing {
            version: 1,
            registration: self
                .registration
                .stored_record()
                .map_err(|_| ProbeError::SessionStoreFailed)?
                .to_vec(),
            pairing: self.pairing.stored_record().to_vec(),
        };
        let encoded = crate::session_store::SessionRecord::new(record.encode_to_vec())
            .map_err(|_| ProbeError::SessionStoreFailed)?;
        let key = self
            .registration
            .store_key()
            .map_err(|_| ProbeError::SessionStoreFailed)?;
        store
            .confirmed_store()
            .map_err(|_| ProbeError::SessionStoreFailed)?
            .store(&key, &encoded)
            .map_err(|_| ProbeError::SessionStoreFailed)
    }

    /// Restore pairing evidence and keys without claiming a live connection.
    /// An expired transport token remains expired and requires separate refresh.
    pub fn restore_all(store: &SessionStore) -> Result<Vec<Self>, ProbeError> {
        let mut restored = Vec::new();
        for (key, record) in store
            .confirmed_store()
            .map_err(|_| ProbeError::SessionStoreFailed)?
            .load_all()
            .map_err(|_| ProbeError::SessionStoreFailed)?
        {
            let saved = StoredConfirmedPairing::decode(record.as_bytes())
                .map_err(|_| ProbeError::SessionStoreFailed)?;
            if saved.version != 1 {
                return Err(ProbeError::SessionStoreFailed);
            }
            let registration =
                UnpairedRegistration::restore_confirmed_registration(&saved.registration)
                    .map_err(|_| ProbeError::SessionStoreFailed)?;
            if registration
                .store_key()
                .map_err(|_| ProbeError::SessionStoreFailed)?
                != key
            {
                return Err(ProbeError::SessionStoreFailed);
            }
            let pairing = PhoneConfirmedPairing::restore_record(&saved.pairing)
                .map_err(|_| ProbeError::SessionStoreFailed)?;
            if restored
                .iter()
                .any(|existing: &Self| existing.account_id() == registration.handover_account_id())
            {
                return Err(ProbeError::SessionStoreFailed);
            }
            restored.push(Self {
                registration,
                pairing,
            });
        }
        Ok(restored)
    }

    /// Forget local credentials. This does not assert remote revocation.
    pub fn forget(&self, store: &SessionStore) -> Result<(), ProbeError> {
        let key = self
            .registration
            .store_key()
            .map_err(|_| ProbeError::SessionStoreFailed)?;
        // Keep confirmed evidence if removal fails partway through.
        store
            .delete(&key)
            .map_err(|_| ProbeError::SessionStoreFailed)?;
        store
            .confirmed_store()
            .map_err(|_| ProbeError::SessionStoreFailed)?
            .delete(&key)
            .map_err(|_| ProbeError::SessionStoreFailed)?;
        Ok(())
    }

    pub fn account_id(&self) -> &str {
        self.registration.handover_account_id()
    }

    /// Offline channel operations retain keys inside the protocol client.
    pub fn encrypt(
        &self,
        payload: &[u8],
    ) -> Result<crate::pairing::cipher::EncryptedPayload, crate::pairing::cipher::CipherError> {
        self.pairing.encrypt_payload(payload)
    }
}

#[derive(Debug)]
pub enum LoginOutcome {
    Ready,
    PhoneConfirmed {
        pairing: Box<ConfirmedPairing>,
        acknowledgement_accepted: bool,
    },
}

/// The symbol is the only ceremony material that may cross the helper boundary.
#[derive(Debug, PartialEq, Eq)]
pub enum LoginProgress {
    Ready,
    Verification(String),
}

impl LoginBootstrap {
    /// Reject arbitrary bundles, unknown aliases, and invalid browser proofs
    /// before any network operation. A valid alias must name one saved record.
    pub fn from_bundle(
        account: &str,
        bundle_b64: &str,
        store: &SessionStore,
    ) -> Result<Self, ProbeError> {
        check_account_id(account).map_err(|_| ProbeError::InvalidBootstrap)?;
        if bundle_b64.is_empty() || bundle_b64.len() > MAX_BUNDLE_BYTES {
            return Err(ProbeError::InvalidBootstrap);
        }
        let decoded = Zeroizing::new(
            general_purpose::STANDARD
                .decode(bundle_b64)
                .map_err(|_| ProbeError::InvalidBootstrap)?,
        );
        let body = decoded
            .strip_prefix(LOGIN_MAGIC)
            .ok_or(ProbeError::InvalidBootstrap)?;
        let mut proof: BrowserProof =
            serde_json::from_slice(body).map_err(|_| ProbeError::InvalidBootstrap)?;
        proof.validate_login()?;
        let start_pairing = proof.kind == "gaia_pairing_start";
        let mut matches = UnpairedRegistration::restore_all_pending(store)
            .map_err(|_| ProbeError::RegistrationFailed)?
            .into_iter()
            .filter(|registration| registration.handover_account_id() == account);
        let registration = matches.next().ok_or(ProbeError::NoPendingRegistration)?;
        if matches.next().is_some() {
            return Err(ProbeError::AmbiguousRegistration);
        }
        if start_pairing
            && store
                .confirmed_store()
                .map_err(|_| ProbeError::SessionStoreFailed)?
                .load(
                    &registration
                        .store_key()
                        .map_err(|_| ProbeError::SessionStoreFailed)?,
                )
                .map_err(|_| ProbeError::SessionStoreFailed)?
                .is_some()
        {
            return Err(ProbeError::InvalidBootstrap);
        }
        proof.kind = "gaia_pairing".to_owned();
        Ok(Self {
            proof,
            registration,
            start_pairing,
            store: store.clone(),
        })
    }

    /// One attempt, no retry. `gaia_login` performs only source lookup and
    /// account binding. Only an explicit `gaia_pairing_start` begins the ceremony.
    pub async fn run(
        self,
        progress: impl FnMut(LoginProgress) -> Result<(), ProbeError>,
    ) -> Result<LoginOutcome, ProbeError> {
        let transport = PairingHttp {
            short: crate::client(true)?,
            stream: crate::streaming_client()?,
            endpoint: self.proof.endpoint.clone(),
        };
        let deadline = MAX_PAIRING_TIME.min(
            self.registration
                .remaining_lifetime()
                .map_err(|_| ProbeError::SessionExpired)?,
        );
        tokio::time::timeout(deadline, self.run_with_transport(transport, progress))
            .await
            .map_err(|_| ProbeError::Timeout)?
    }

    async fn run_with_transport(
        self,
        http: PairingHttp,
        mut progress: impl FnMut(LoginProgress) -> Result<(), ProbeError>,
    ) -> Result<LoginOutcome, ProbeError> {
        let Self {
            proof,
            registration,
            start_pairing,
            store,
        } = self;
        let session_id = Uuid::new_v4().to_string();
        let sources = crate::query_response(
            &http.short,
            &http.url(crate::SIGN_IN_PATH),
            proof.validate_pairing()?,
            &crate::lookup_request(),
            false,
        )
        .await?;
        // Google's a5a compares the saved web registration identity against
        // the account's source list. Never route its token to another account.
        let prepared = crate::prepare_pairing_from_sources(
            proof
                .account_email
                .as_deref()
                .ok_or(ProbeError::InvalidCredentials)?,
            &registration,
            &session_id,
            &sources,
        )?;
        if !start_pairing {
            progress(LoginProgress::Ready)?;
            return Ok(LoginOutcome::Ready);
        }
        let request = registration
            .prepare_receive()
            .map_err(|_| ProbeError::SessionExpired)?;
        let (reply_tx, mut replies) = mpsc::channel(RECEIVE_QUEUE);
        let (ready_tx, ready_rx) = oneshot::channel();
        let receive_url = http.url(crate::RECEIVE_MESSAGES_PATH);
        let receive = crate::receive_stream_when_ready(
            &http.stream,
            &receive_url,
            proof.validate_messaging()?,
            &request,
            move |event| {
                match event {
                    ReceiveEvent::Record(record) => {
                        if let Some(reply) = record.pairing_reply()? {
                            reply_tx
                                .try_send(reply)
                                .map_err(|_| ReceiveError::TooLarge)?;
                        }
                    }
                    ReceiveEvent::Status(status) if status != 0 => {
                        return Err(ReceiveError::Failed);
                    }
                    ReceiveEvent::Status(_) => {}
                }
                Ok(())
            },
            move || {
                let _ = ready_tx.send(());
            },
        );
        tokio::pin!(receive);
        tokio::select! {
            _ = &mut receive => return Err(ProbeError::ReceiveFailed),
            ready = ready_rx => { ready.map_err(|_| ProbeError::ReceiveFailed)?; }
        }
        tokio::select! {
            _ = &mut receive => return Err(ProbeError::ReceiveFailed),
            sent = http.send(&proof, prepared.envelope()) => { sent?; }
        }
        let first = wait_reply(receive.as_mut(), &mut replies, |reply| {
            prepared.matches_reply(reply)
        })
        .await?;
        let (pending, ack) = prepared.accept_reply(first)?;
        let symbol = pending
            .verification_emoji()
            .map_err(|_| ProbeError::PairingFailed)?;
        progress(LoginProgress::Verification(symbol.as_str().to_owned()))?;
        let final_envelope = registration
            .confirmation_envelope(
                &pending,
                proof
                    .account_email
                    .as_deref()
                    .ok_or(ProbeError::InvalidCredentials)?,
                &session_id,
            )
            .map_err(|_| ProbeError::PairingFailed)?;
        tokio::select! {
            _ = &mut receive => return Err(ProbeError::ReceiveFailed),
            acked = http.ack(&proof, &registration, ack) => { acked?; }
        }
        tokio::select! {
            _ = &mut receive => return Err(ProbeError::ReceiveFailed),
            sent = http.send(&proof, &final_envelope) => { sent?; }
        }
        let final_reply = wait_reply(receive.as_mut(), &mut replies, |reply| {
            reply.matches_confirmation(&pending)
        })
        .await?;
        let (pairing, ack) = final_reply
            .accept_confirmation_and_ack(pending)
            .map_err(|_| ProbeError::PairingFailed)?;
        let confirmed = ConfirmedPairing {
            registration,
            pairing,
        };
        // Save before ACK. Once the phone confirms, an ACK outage must not
        // destroy the keys or invite another pairing attempt.
        confirmed.persist(&store)?;
        let acknowledgement_accepted = tokio::select! {
            _ = &mut receive => false,
            acked = http.ack(&proof, &confirmed.registration, ack) => acked.is_ok(),
        };
        Ok(LoginOutcome::PhoneConfirmed {
            pairing: Box::new(confirmed),
            acknowledgement_accepted,
        })
    }
}

async fn wait_reply(
    mut receive: std::pin::Pin<&mut impl std::future::Future<Output = Result<u8, ProbeError>>>,
    replies: &mut mpsc::Receiver<PairingReply>,
    matches: impl Fn(&PairingReply) -> bool,
) -> Result<PairingReply, ProbeError> {
    loop {
        tokio::select! {
            _ = &mut receive => return Err(ProbeError::ReceiveFailed),
            reply = replies.recv() => {
                let reply = reply.ok_or(ProbeError::ReceiveFailed)?;
                if matches(&reply) { return Ok(reply); }
            }
        }
    }
}

struct PairingHttp {
    short: reqwest::Client,
    stream: reqwest::Client,
    endpoint: String,
}

impl PairingHttp {
    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.endpoint)
    }

    async fn send(
        &self,
        proof: &BrowserProof,
        envelope: &crate::pairing::gaia::PairingSendEnvelope,
    ) -> Result<(), ProbeError> {
        envelope
            .ensure_valid()
            .map_err(|_| ProbeError::SessionExpired)?;
        let mut headers = proof.validate_pairing()?;
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/x-protobuf"),
        );
        crate::post_pairing_envelope(
            &self.short,
            &self.url(crate::SEND_MESSAGE_PATH),
            headers,
            envelope,
        )
        .await?;
        Ok(())
    }

    async fn ack(
        &self,
        proof: &BrowserProof,
        registration: &UnpairedRegistration,
        ack: Acknowledgement,
    ) -> Result<(), ProbeError> {
        let mut batch = AckBatch::default();
        batch.push(ack).map_err(|_| ProbeError::ReceiveFailed)?;
        let request = registration
            .acknowledgement_request(&batch)
            .map_err(|_| ProbeError::SessionExpired)?;
        let mut headers = proof.validate_messaging()?;
        headers.insert(
            reqwest::header::CONTENT_TYPE,
            reqwest::header::HeaderValue::from_static("application/x-protobuf"),
        );
        crate::post_acknowledgements(
            &self.short,
            &self.url(crate::ACK_MESSAGES_PATH),
            headers,
            &request,
        )
        .await?;
        Ok(())
    }
}

// Private local file schema. Cookies and account email are deliberately absent.
#[derive(Message)]
struct StoredConfirmedPairing {
    #[prost(uint32, tag = "1")]
    version: u32,
    #[prost(bytes = "vec", tag = "2")]
    registration: Vec<u8>,
    #[prost(bytes = "vec", tag = "3")]
    pairing: Vec<u8>,
}

impl Drop for StoredConfirmedPairing {
    fn drop(&mut self) {
        self.registration.zeroize();
        self.pairing.zeroize();
    }
}

#[cfg(test)]
mod tests;
