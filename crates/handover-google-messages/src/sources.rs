//! Read-only registration selection. Identifiers stay inside the protocol client.

use base64::{Engine, engine::general_purpose};
use prost::Message;
use serde_json::Value;
use std::fmt;

use crate::{ProbeError, lookup_records};

const ID_LIMIT: usize = 1024;
const METADATA_LIMIT: usize = 4096;

/// A bounded inventory from one lookup. It is not a paired session.
pub struct RegisteredSources {
    total: usize,
    phones: Vec<RegisteredPhone>,
}

impl fmt::Debug for RegisteredSources {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RegisteredSources { redacted }")
    }
}

/// Only observed phone metadata is retained. No keys or arbitrary source fields.
pub struct RegisteredPhone {
    identity: Vec<u8>,
    enabled: bool,
    registration_time: i64,
    last_refresh_micros: Option<u64>,
    prewarm_supported: bool,
}

impl fmt::Debug for RegisteredPhone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("RegisteredPhone { redacted }")
    }
}

/// Selection does not register, connect, or attest that a phone is online.
#[derive(Debug)]
pub enum PhoneSelection<'a> {
    NoneEligible,
    Ambiguous,
    Selected(&'a RegisteredPhone),
}

impl RegisteredSources {
    pub fn from_lookup_response(body: &[u8]) -> Result<Self, ProbeError> {
        let records = lookup_records(body)?;
        let mut phones = Vec::new();
        for record in &records {
            let fields = record.as_array().ok_or(ProbeError::UnexpectedResponse)?;
            // Google's xJ filters source field 3 to types 1 and 4.
            let Some(kind) = fields.get(2).and_then(Value::as_u64) else {
                continue;
            };
            if !matches!(kind, 1 | 4) {
                continue;
            }
            let identity = bytes(fields.first(), ID_LIMIT)?;
            if identity.is_empty() {
                return Err(ProbeError::UnexpectedResponse);
            }
            let metadata = bytes(fields.get(7), METADATA_LIMIT)?;
            let metadata = PhoneMetadata::decode(metadata.as_slice())
                .map_err(|_| ProbeError::UnexpectedResponse)?;
            let last_refresh_micros = match fields.get(6) {
                None | Some(Value::Null) => None,
                Some(Value::String(s))
                    if !s.is_empty() && s.len() <= 20 && s.bytes().all(|b| b.is_ascii_digit()) =>
                {
                    Some(
                        s.parse::<u64>()
                            .map_err(|_| ProbeError::UnexpectedResponse)?,
                    )
                }
                Some(value) => Some(value.as_u64().ok_or(ProbeError::UnexpectedResponse)?),
            };
            if phones
                .iter()
                .any(|p: &RegisteredPhone| p.identity == identity)
            {
                return Err(ProbeError::UnexpectedResponse);
            }
            phones.push(RegisteredPhone {
                identity,
                enabled: metadata.enabled,
                registration_time: metadata.registration_time,
                last_refresh_micros,
                prewarm_supported: metadata.prewarm_supported,
            });
        }
        Ok(Self {
            total: records.len(),
            phones,
        })
    }

    pub fn total_count(&self) -> usize {
        self.total
    }

    pub fn phone_count(&self) -> usize {
        self.phones.len()
    }

    pub fn select_phone(&self) -> PhoneSelection<'_> {
        let mut selected: Option<&RegisteredPhone> = None;
        let mut ambiguous = false;
        for phone in &self.phones {
            if !phone.enabled || phone.registration_time <= 0 {
                continue;
            }
            match selected {
                Some(current) if phone.registration_time < current.registration_time => {}
                Some(current) if phone.registration_time == current.registration_time => {
                    ambiguous = true;
                }
                _ => {
                    selected = Some(phone);
                    ambiguous = false;
                }
            }
        }
        match selected {
            None => PhoneSelection::NoneEligible,
            Some(_) if ambiguous => PhoneSelection::Ambiguous,
            Some(phone) => PhoneSelection::Selected(phone),
        }
    }
}

impl RegisteredPhone {
    pub fn last_refresh_micros(&self) -> Option<u64> {
        self.last_refresh_micros
    }

    pub fn prewarm_supported(&self) -> bool {
        self.prewarm_supported
    }
}

// Partial, independently authored projection of Gza/Hza. Field 3 may contain
// key material and is deliberately skipped by prost, never retained here.
#[derive(Message)]
struct PhoneMetadata {
    #[prost(bool, tag = "1")]
    enabled: bool,
    #[prost(int64, tag = "2")]
    registration_time: i64,
    #[prost(bool, tag = "4")]
    prewarm_supported: bool,
}

fn bytes(value: Option<&Value>, limit: usize) -> Result<Vec<u8>, ProbeError> {
    let encoded = match value {
        None | Some(Value::Null) => return Ok(Vec::new()),
        Some(Value::String(s)) if s.len() <= limit.div_ceil(3) * 4 => s,
        _ => return Err(ProbeError::UnexpectedResponse),
    };
    for engine in [
        general_purpose::STANDARD,
        general_purpose::STANDARD_NO_PAD,
        general_purpose::URL_SAFE,
        general_purpose::URL_SAFE_NO_PAD,
    ] {
        if let Ok(decoded) = engine.decode(encoded) {
            if decoded.len() <= limit {
                return Ok(decoded);
            }
        }
    }
    Err(ProbeError::UnexpectedResponse)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn phone(id: &str, kind: u64, enabled: bool, time: i64) -> Value {
        let mut metadata = PhoneMetadata {
            enabled,
            registration_time: time,
            prewarm_supported: true,
        }
        .encode_to_vec();
        // Unknown field 3 contains synthetic secret bytes. The projection skips it.
        metadata.extend_from_slice(&[0x1a, 6, b's', b'e', b'c', b'r', b'e', b't']);
        json!([
            general_purpose::STANDARD.encode(id),
            null,
            kind,
            null,
            null,
            null,
            "18446744073709551615",
            general_purpose::STANDARD.encode(metadata)
        ])
    }

    fn inventory(records: Vec<Value>) -> Result<RegisteredSources, ProbeError> {
        RegisteredSources::from_lookup_response(
            &serde_json::to_vec(&json!([[], null, [null, null, records, null]])).unwrap(),
        )
    }

    #[test]
    fn selects_newest_enabled_phone_and_discards_key_material() {
        let sources = inventory(vec![
            phone("older", 1, true, 10),
            phone("newer", 4, true, 20),
            phone("disabled", 1, false, 30),
            phone("web", 3, true, 40),
        ])
        .unwrap();
        assert_eq!(sources.total_count(), 4);
        assert_eq!(sources.phone_count(), 3);
        let PhoneSelection::Selected(phone) = sources.select_phone() else {
            panic!()
        };
        assert_eq!(phone.identity, b"newer");
        assert_eq!(phone.last_refresh_micros(), Some(u64::MAX));
        assert!(phone.prewarm_supported());
        assert_eq!(format!("{sources:?}"), "RegisteredSources { redacted }");
        assert_eq!(format!("{phone:?}"), "RegisteredPhone { redacted }");
    }

    #[test]
    fn equal_newest_times_are_ambiguous_in_either_order() {
        for ids in [["a", "b"], ["b", "a"]] {
            let sources = inventory(ids.map(|id| phone(id, 1, true, 20)).to_vec()).unwrap();
            assert!(matches!(sources.select_phone(), PhoneSelection::Ambiguous));
        }
        let sources = inventory(vec![
            phone("a", 1, true, 20),
            phone("b", 1, true, 20),
            phone("c", 4, true, 30),
        ])
        .unwrap();
        assert!(matches!(
            sources.select_phone(),
            PhoneSelection::Selected(_)
        ));
    }

    #[test]
    fn missing_metadata_never_invents_eligibility() {
        for record in [
            phone("disabled", 1, false, 10),
            phone("no-time", 4, true, 0),
            json!(["aWQ=", null, 1]),
        ] {
            assert!(matches!(
                inventory(vec![record]).unwrap().select_phone(),
                PhoneSelection::NoneEligible
            ));
        }
    }

    #[test]
    fn rejects_duplicate_ids_malformed_metadata_and_unbounded_inputs() {
        assert!(inventory(vec![phone("same", 1, true, 1), phone("same", 4, true, 2)]).is_err());
        let mut bad = phone("id", 1, true, 1);
        bad[7] = json!("not-base64!");
        assert!(inventory(vec![bad]).is_err());
        let mut large = phone("id", 1, true, 1);
        large[0] = json!(general_purpose::STANDARD.encode(vec![0; ID_LIMIT + 1]));
        assert!(inventory(vec![large]).is_err());
        assert!(inventory(vec![json!([]); 129]).is_err());
        assert!(
            RegisteredSources::from_lookup_response(&vec![b' '; crate::RESPONSE_LIMIT + 1])
                .is_err()
        );
    }
}
