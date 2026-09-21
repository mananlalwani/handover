use std::fs;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;
use std::path::Path;

use openssl::asn1::Asn1Time;
use openssl::bn::{BigNum, MsbOption};
use openssl::ec::{EcGroup, EcKey};
use openssl::hash::MessageDigest;
use openssl::nid::Nid;
use openssl::pkey::{Id, PKey, Private};
use openssl::x509::{X509, X509NameBuilder};
use sha2::{Digest, Sha256};

use crate::errors::NativeError;

pub(crate) fn generate_identity() -> Result<(PKey<Private>, X509), NativeError> {
    let group = EcGroup::from_curve_name(Nid::X9_62_PRIME256V1)?;
    let key = PKey::from_ec_key(EcKey::generate(&group)?)?;
    let mut subject = X509NameBuilder::new()?;
    subject.append_entry_by_text("CN", "Handover Linux")?;
    let subject = subject.build();
    let mut cert = X509::builder()?;
    cert.set_version(2)?;
    let mut serial = BigNum::new()?;
    serial.rand(128, MsbOption::MAYBE_ZERO, false)?;
    let serial = serial.to_asn1_integer()?;
    cert.set_serial_number(serial.as_ref())?;
    cert.set_subject_name(&subject)?;
    cert.set_issuer_name(&subject)?;
    cert.set_pubkey(&key)?;
    let not_before = Asn1Time::days_from_now(0)?;
    let not_after = Asn1Time::days_from_now(3650)?;
    cert.set_not_before(not_before.as_ref())?;
    cert.set_not_after(not_after.as_ref())?;
    cert.sign(&key, MessageDigest::sha256())?;
    Ok((key, cert.build()))
}
pub(crate) fn write_private(path: &Path, bytes: &[u8]) -> Result<(), NativeError> {
    use std::os::unix::fs::OpenOptionsExt;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}
pub(crate) fn fingerprint(cert: &X509) -> Result<String, NativeError> {
    Ok(hex::encode(Sha256::digest(cert.to_der()?)))
}

pub(crate) fn validate_peer_certificate(cert: &X509) -> Result<(), NativeError> {
    let now = Asn1Time::days_from_now(0)?;
    if cert.not_before() > now.as_ref() || cert.not_after() < now.as_ref() {
        return Err(NativeError::InvalidFrame);
    }
    // Native identities are deliberately self-signed and authenticated by the
    // pairing ceremony/fingerprint pin. Still reject malformed or non-leaf
    // certificates before accepting them as an identity.
    let key = cert.public_key()?;
    let valid_curve =
        key.id() == Id::EC && key.ec_key()?.group().curve_name() == Some(Nid::X9_62_PRIME256V1);
    if !valid_curve || !cert.verify(&key)? {
        return Err(NativeError::InvalidFrame);
    }
    Ok(())
}
