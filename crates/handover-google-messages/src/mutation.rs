//! Independently observed Gaia phone mutations, never inferred from HTTP acceptance.
use crate::{ProbeError, session::SessionError};
use handover_core::messaging::{MAX_EMOJI_CHARS, MAX_ID_LEN};
use prost::Message;
use zeroize::{Zeroize, Zeroizing};

fn invalid() -> ProbeError {
    ProbeError::SessionProtocol(SessionError::SendModel)
}
fn identifier(value: &str) -> Result<(), ProbeError> {
    if value.is_empty() || value.len() > MAX_ID_LEN || value.chars().any(char::is_control) {
        return Err(invalid());
    }
    Ok(())
}
pub(crate) fn reaction(
    message: &str,
    emoji: &str,
    add: bool,
) -> Result<Zeroizing<Vec<u8>>, ProbeError> {
    identifier(message)?;
    if emoji.is_empty()
        || emoji.chars().count() > MAX_EMOJI_CHARS
        || emoji.chars().any(char::is_control)
    {
        return Err(invalid());
    }
    // Unknown/custom emoji retain their string and use the observed custom enum.
    let kind = match emoji {
        "👍" => 1,
        "😍" => 2,
        "😂" => 3,
        "😮" => 4,
        "😢" => 5,
        "😠" => 6,
        "👎" => 7,
        "😡" => 10,
        "❤️" => 11,
        "😭" => 12,
        _ => 8,
    };
    Ok(Zeroizing::new(
        ReactionRequest {
            message: message.into(),
            reaction: Some(Emoji {
                value: emoji.into(),
                kind,
            }),
            action: if add { 1 } else { 2 },
        }
        .encode_to_vec(),
    ))
}
pub(crate) fn unpair(pairing: &str) -> Result<Zeroizing<Vec<u8>>, ProbeError> {
    uuid::Uuid::parse_str(pairing).map_err(|_| invalid())?;
    Ok(Zeroizing::new(
        UnpairRequest {
            pairing: pairing.into(),
        }
        .encode_to_vec(),
    ))
}
pub(crate) fn response(action: i32, bytes: &[u8]) -> Result<bool, ProbeError> {
    if bytes.len() > 4096 {
        return Err(invalid());
    }
    let result = ResultCode::decode(bytes)
        .map_err(|_| invalid())?
        .result
        .ok_or_else(invalid)?;
    match (action, result) {
        (38, 1) | (46, 1) => Ok(true),
        (38, 2) | (46, 0) => Ok(false),
        _ => Err(invalid()),
    }
}
#[derive(Message)]
#[prost(skip_debug)]
struct ReactionRequest {
    #[prost(string, tag = "1")]
    message: String,
    #[prost(message, optional, tag = "2")]
    reaction: Option<Emoji>,
    #[prost(int32, tag = "3")]
    action: i32,
}
impl Drop for ReactionRequest {
    fn drop(&mut self) {
        self.message.zeroize();
    }
}
#[derive(Message)]
#[prost(skip_debug)]
struct Emoji {
    #[prost(string, tag = "1")]
    value: String,
    #[prost(int32, tag = "2")]
    kind: i32,
}
impl Drop for Emoji {
    fn drop(&mut self) {
        self.value.zeroize();
    }
}
#[derive(Message)]
#[prost(skip_debug)]
struct UnpairRequest {
    #[prost(string, tag = "1")]
    pairing: String,
}
impl Drop for UnpairRequest {
    fn drop(&mut self) {
        self.pairing.zeroize();
    }
}
#[derive(Message)]
struct ResultCode {
    #[prost(int32, optional, tag = "1")]
    result: Option<i32>,
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reaction_wire_fixture_and_rejection() {
        assert_eq!(
            reaction("m", "👍", true).unwrap().as_slice(),
            b"\x0a\x01m\x12\x08\x0a\x04\xf0\x9f\x91\x8d\x10\x01\x18\x01"
        );
        let remove =
            ReactionRequest::decode(reaction("m", "👍", false).unwrap().as_slice()).unwrap();
        assert_eq!(remove.action, 2);
        assert!(reaction("", "👍", true).is_err());
        assert!(reaction("m", "\n", true).is_err());
    }
    #[test]
    fn only_explicit_phone_results_establish_effect() {
        for action in [38, 46] {
            assert!(response(action, &[8, 1]).unwrap());
            assert!(response(action, &[]).is_err());
            assert!(response(action, &[8, 3]).is_err());
            assert!(response(action, &[8]).is_err());
        }
        assert!(!response(38, &[8, 2]).unwrap());
        assert!(!response(46, &[8, 0]).unwrap());
        assert!(unpair("source-not-a-pairing-attempt").is_err());
    }
}
