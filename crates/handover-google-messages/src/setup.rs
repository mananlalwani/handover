//! Explicit local setup using the native client's existing registration and pairing.

use std::time::Duration;

use base64::{Engine, engine::general_purpose};
use zeroize::Zeroizing;

use crate::{
    BrowserProof, ProbeError,
    credential_store::DesktopCredentialStore,
    login::{ConfirmedPairing, LoginBootstrap, LoginOutcome, LoginProgress},
    register_device,
    session_store::SessionStore,
};

/// Reauthenticate a uniquely matching saved account, otherwise register and
/// pair a fresh device once. No failed or interrupted operation is retried.
pub async fn connect(
    mut proof: BrowserProof,
    mut progress: impl FnMut(LoginProgress) -> Result<(), ProbeError>,
) -> Result<String, ProbeError> {
    if proof.kind != "gaia_pairing_start" {
        return Err(ProbeError::InvalidBootstrap);
    }
    proof.validate_login()?;
    DesktopCredentialStore::check_writable()
        .await
        .map_err(ProbeError::CredentialStore)?;
    let store = SessionStore::default_store().map_err(|_| ProbeError::SessionStoreFailed)?;
    proof.kind = "gaia_pairing".into();
    let sources = Zeroizing::new(
        crate::query_response(
            &crate::client(true)?,
            &format!("{}{}", proof.endpoint, crate::SIGN_IN_PATH),
            proof.validate_pairing()?,
            &crate::lookup_request(),
            false,
        )
        .await?,
    );
    let sources = crate::sources::RegisteredSources::from_lookup_response(&sources)
        .map_err(|_| ProbeError::RegistrationFailed)?;
    let mut matching = Vec::new();
    for paired in ConfirmedPairing::restore_all(&store)? {
        // Select by attested device and phone identities, not an email label.
        // This also permits recovery when the old browser authentication is gone.
        if sources.contains_registration(&paired.registration)
            && sources.contains_paired_phone(paired.pairing.peer())
        {
            matching.push(paired);
        }
    }
    if matching.len() > 1 {
        return Err(ProbeError::AmbiguousRegistration);
    }
    let account = if let Some(mut paired) = matching.pop() {
        if paired.registration.remaining_lifetime().is_err() {
            paired.registration = crate::renew_registration(&proof, &paired.registration).await?;
            // Pin and save the exact renewed identity before any further query.
            paired.persist(&store)?;
        }
        let sources = Zeroizing::new(
            crate::query_response(
                &crate::client(true)?,
                &format!("{}{}", proof.endpoint, crate::SIGN_IN_PATH),
                proof.validate_pairing()?,
                &paired.registration.lookup_request(),
                false,
            )
            .await?,
        );
        let sources = crate::sources::RegisteredSources::from_lookup_response(&sources)
            .map_err(|_| ProbeError::RegistrationFailed)?;
        if !sources.contains_registration(&paired.registration)
            || !sources.contains_paired_phone(paired.pairing.peer())
        {
            return Err(ProbeError::RegistrationFailed);
        }
        paired
            .registration
            .persist_pending(&store)
            .map_err(|_| ProbeError::SessionStoreFailed)?;
        let account = paired.account_id().to_owned();
        proof.kind = "gaia_login".into();
        account
    } else {
        let email = proof.account_email.take();
        proof.kind = "gaia_register".into();
        let registration = register_device(&proof, Duration::from_secs(30 * 24 * 60 * 60)).await?;
        let account = registration.handover_account_id().to_owned();
        registration
            .persist_pending(&store)
            .map_err(|_| ProbeError::SessionStoreFailed)?;
        proof.account_email = email;
        proof.kind = "gaia_pairing_start".into();
        account
    };
    let mut encoded = Zeroizing::new(b"HOVL\x01\0".to_vec());
    encoded
        .extend_from_slice(&serde_json::to_vec(&proof).map_err(|_| ProbeError::InvalidBootstrap)?);
    let bundle = Zeroizing::new(general_purpose::STANDARD.encode(encoded.as_slice()));
    let bootstrap = LoginBootstrap::from_bundle(&account, &bundle, &store)?;
    match bootstrap.run(&mut progress).await? {
        LoginOutcome::CredentialsSaved => Ok(account),
        LoginOutcome::PhoneConfirmed {
            credential_error: Some(error),
            ..
        } => Err(ProbeError::CredentialStore(error)),
        LoginOutcome::PhoneConfirmed { .. } => Ok(account),
        LoginOutcome::Ready => Err(ProbeError::NativeError),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn other_proof_modes_cannot_register_pair_or_store_credentials() {
        for kind in [
            "gaia_lookup_with_cookies",
            "gaia_register",
            "gaia_login",
            "gaia_pairing",
        ] {
            let proof = BrowserProof {
                kind: kind.into(),
                endpoint: "https://instantmessaging-pa.googleapis.com".into(),
                origin: "https://messages.google.com".into(),
                authorization: "Bearer synthetic".into(),
                api_key: "synthetic".into(),
                auth_user: None,
                service_cookie: Some("SID=synthetic".into()),
                account_email: Some("person@example.test".into()),
                browser_request: None,
            };
            assert!(matches!(
                connect(proof, |_| panic!("unexpected pairing progress")).await,
                Err(ProbeError::InvalidBootstrap)
            ));
        }
    }
}
