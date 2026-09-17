//! Read-only local audio inspection. Never changes profiles, defaults or links.
//! This is host-wide, not an inferred association with a native phone identity.
use handover_core::CallAudioStatus;
use serde_json::Value;
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;

const LIMIT: u64 = 1024 * 1024;

async fn query(kind: &str) -> Option<Value> {
    tokio::time::timeout(Duration::from_secs(3), async {
        let mut child = tokio::process::Command::new("pactl")
            .args(["--format=json", "list", kind])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .ok()?;
        let mut data = Vec::new();
        child
            .stdout
            .take()?
            .take(LIMIT + 1)
            .read_to_end(&mut data)
            .await
            .ok()?;
        if data.len() as u64 > LIMIT || !child.wait().await.ok()?.success() {
            return None;
        }
        serde_json::from_slice(&data).ok()
    })
    .await
    .ok()
    .flatten()
}

pub(crate) async fn inspect() -> CallAudioStatus {
    let (cards, sources, sinks) = tokio::join!(query("cards"), query("sources"), query("sinks"));
    match (cards, sources, sinks) {
        (Some(cards), Some(sources), Some(sinks)) => normalize(&cards, &sources, &sinks),
        _ => CallAudioStatus::default(),
    }
}

fn normalize(cards: &Value, sources: &Value, sinks: &Value) -> CallAudioStatus {
    let (Some(cards), Some(sources), Some(sinks)) =
        (cards.as_array(), sources.as_array(), sinks.as_array())
    else {
        return CallAudioStatus::default();
    };
    let gateways: Vec<_> = cards
        .iter()
        .filter(|card| card["active_profile"] == "audio-gateway")
        .collect();
    let duplex = gateways.iter().any(|card| {
        let Some(index) = card["index"].as_u64() else {
            return false;
        };
        let running =
            |node: &&Value| node["card"].as_u64() == Some(index) && node["state"] == "RUNNING";
        sources
            .iter()
            .filter(|node| node["monitor_of_sink"].is_null())
            .any(|node| running(&node))
            && sinks.iter().any(|node| running(&node))
    });
    CallAudioStatus {
        observed: true,
        gateway_ready: !gateways.is_empty(),
        duplex_running: duplex,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn readiness_is_not_routing_confirmation() {
        let cards = json!([{"index":7,"active_profile":"audio-gateway"}]);
        assert_eq!(
            normalize(&cards, &json!([]), &json!([])),
            CallAudioStatus {
                observed: true,
                gateway_ready: true,
                duplex_running: false,
            }
        );
        let sources = json!([{"card":7,"state":"RUNNING","monitor_of_sink":null}]);
        assert!(normalize(&cards, &sources, &json!([{"card":7,"state":"RUNNING"}])).duplex_running);
        assert!(
            !normalize(&cards, &sources, &json!([{"card":8,"state":"RUNNING"}])).duplex_running
        );
        assert!(
            !normalize(
                &cards,
                &json!([{"card":7,"state":"RUNNING","monitor_of_sink":3}]),
                &json!([{"card":7,"state":"RUNNING"}])
            )
            .duplex_running
        );
        assert!(!normalize(&Value::Null, &json!([]), &json!([])).observed);
    }
}
