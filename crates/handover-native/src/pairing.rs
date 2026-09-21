use sha2::{Digest, Sha256};

pub(crate) fn comparison_code(
    own_fp: &str,
    own_nonce: &str,
    peer_fp: &str,
    peer_nonce: &str,
) -> String {
    // Each nonce stays bound to its fingerprint owner, so both sides derive
    // the same code without roles while a middlebox cannot swap openings.
    let ((low_fp, low_nonce), (high_fp, high_nonce)) = if own_fp <= peer_fp {
        ((own_fp, own_nonce), (peer_fp, peer_nonce))
    } else {
        ((peer_fp, peer_nonce), (own_fp, own_nonce))
    };
    let digest = Sha256::digest(
        format!("handover-pair-v2:{low_fp}:{high_fp}:{low_nonce}:{high_nonce}").as_bytes(),
    );
    let number = u32::from_be_bytes(digest[..4].try_into().unwrap()) % 100_000_000;
    format!("{number:08}")
}
pub(crate) fn parse_nonce(hex_nonce: &str) -> Option<[u8; 16]> {
    hex::decode(hex_nonce).ok()?.try_into().ok()
}
