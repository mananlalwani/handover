//! Helper IPC fixtures cover the legacy adapter's original records and native
//! helper extensions. New native records are authored here. The tests check
//! event decoding, generation grouping, cursors, status tokens, and tolerance
//! for unknown future keys.

use handover_gmessages::contract::HelperEvent;
use handover_gmessages::normalize::parse_status;

fn fixture() -> Vec<HelperEvent> {
    let raw = include_str!("fixtures/helper-events.jsonl");
    raw.lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| {
            serde_json::from_str(line)
                .unwrap_or_else(|error| panic!("fixture line must decode: {line}\n{error}"))
        })
        .collect()
}

#[test]
fn helper_events_decode_to_known_shapes() {
    let events = fixture();
    assert_eq!(events.len(), 21);
    assert!(matches!(
        events[18],
        HelperEvent::Diagnostic {
            code: handover_gmessages::contract::HelperDiagnostic::TypingRegistered
        }
    ));
    assert!(matches!(events[0], HelperEvent::Hello { .. }));
    assert!(matches!(events[1], HelperEvent::Account { .. }));
    assert!(matches!(
        &events[10],
        HelperEvent::SendStatus { request_id, status, .. }
            if request_id == "r-send-1" && status == "accepted"
    ));
    assert!(matches!(
        events[events.len() - 2],
        HelperEvent::AccountRemoved { .. }
    ));
}

#[test]
fn generation_chunks_group_with_authority_on_close() {
    let events = fixture();
    let chunks: Vec<&HelperEvent> = events
        .iter()
        .filter(|event| {
            matches!(
                event,
                HelperEvent::Conversations {
                    generation: Some(_),
                    ..
                } | HelperEvent::Messages {
                    generation: Some(_),
                    ..
                }
            )
        })
        .collect();
    // Two conversation chunks sharing generation 11, two message
    // chunks sharing generation 12.
    assert_eq!(chunks.len(), 4);
    for generation in [11, 12] {
        let group: Vec<&&HelperEvent> = chunks
            .iter()
            .filter(|event| match event {
                HelperEvent::Conversations { generation: g, .. }
                | HelperEvent::Messages { generation: g, .. } => *g == Some(generation),
                _ => false,
            })
            .collect();
        assert_eq!(group.len(), 2, "generation {generation} must pair up");
        let full_flags: Vec<bool> = group
            .iter()
            .map(|event| match event {
                HelperEvent::Conversations { full, .. } | HelperEvent::Messages { full, .. } => {
                    *full
                }
                _ => false,
            })
            .collect();
        assert_eq!(
            full_flags,
            vec![false, true],
            "only the closing chunk of generation {generation} is authoritative"
        );
    }
}

#[test]
fn status_tokens_parse_to_known_states() {
    let events = fixture();
    let mut statuses: Vec<&str> = events
        .iter()
        .filter_map(|event| match event {
            HelperEvent::Status { status, .. } => Some(status.as_str()),
            _ => None,
        })
        .collect();
    statuses.sort_unstable();
    assert_eq!(statuses, vec!["accepted", "failed:transport", "sent"]);
    for status in statuses {
        parse_status(status)
            .unwrap_or_else(|error| panic!("fixture status must parse: {status}\n{error}"));
    }
}

#[test]
fn unknown_future_keys_are_tolerated() {
    // The final probe line carries `future_flag`, which no released
    // contract defines, and an empty message list no other fixture
    // event has. Decoding must ignore the key rather than fail.
    let events = fixture();
    assert!(
        events.iter().any(|event| matches!(
            event,
            HelperEvent::Messages { messages, .. } if messages.is_empty()
        )),
        "forward-compatibility probe must decode"
    );
}
