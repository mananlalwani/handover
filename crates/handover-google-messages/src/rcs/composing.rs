//! RFC 3994 composing content, bounded before XML parsing.
use std::time::Duration;

const NS: &str = "urn:ietf:params:xml:ns:im-iscomposing";
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Composing {
    Active(Duration),
    Idle,
}

pub(super) fn decode(bytes: &[u8]) -> Result<Composing, ()> {
    if bytes.len() > 8192 {
        return Err(());
    }
    let text = std::str::from_utf8(bytes).map_err(|_| ())?;
    let document = roxmltree::Document::parse_with_options(
        text,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 64,
            ..Default::default()
        },
    )
    .map_err(|_| ())?;
    let root = document.root_element();
    if !root.has_tag_name((NS, "isComposing")) {
        return Err(());
    }
    let mut state = None;
    let mut refresh = None;
    for child in root.children().filter(|node| node.is_element()) {
        if child.tag_name().namespace() != Some(NS) {
            continue;
        }
        if child.children().any(|node| node.is_element()) {
            return Err(());
        }
        match child.tag_name().name() {
            "state" => {
                if state.is_some() {
                    return Err(());
                }
                state = Some(child.text().ok_or(())?.trim());
            }
            "refresh" => {
                if refresh.is_some() {
                    return Err(());
                }
                let seconds = child
                    .text()
                    .ok_or(())?
                    .trim()
                    .parse::<u64>()
                    .map_err(|_| ())?;
                // RFC 3994 permits positive integers. Bound untrusted lifetimes locally.
                if seconds == 0 || seconds > 3600 {
                    return Err(());
                }
                refresh = Some(Duration::from_secs(seconds));
            }
            _ => {}
        }
    }
    match state {
        Some("active") => Ok(Composing::Active(
            refresh.unwrap_or(Duration::from_secs(120)),
        )),
        Some("idle") => Ok(Composing::Idle),
        _ => Err(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn xml(body: &str) -> Vec<u8> {
        format!("<isComposing xmlns=\"{NS}\">{body}</isComposing>").into_bytes()
    }
    #[test]
    fn active_refresh_and_idle_follow_the_composing_document() {
        assert_eq!(
            decode(&xml("<state>active</state><refresh>60</refresh>")),
            Ok(Composing::Active(Duration::from_secs(60)))
        );
        assert_eq!(
            decode(&xml("<state>active</state>")),
            Ok(Composing::Active(Duration::from_secs(120)))
        );
        assert_eq!(decode(&xml("<state>idle</state>")), Ok(Composing::Idle));
        let prefixed = format!(
            "<c:isComposing xmlns:c=\"{NS}\"><c:state>active</c:state><c:refresh>30</c:refresh></c:isComposing>"
        );
        assert_eq!(
            decode(prefixed.as_bytes()),
            Ok(Composing::Active(Duration::from_secs(30)))
        );
    }
    #[test]
    fn rejects_ambiguous_unbounded_and_foreign_signals() {
        for body in [
            "<state>active</state><state>idle</state>",
            "<state>active</state><refresh>0</refresh>",
            "<state>active</state><refresh>99999999999999999</refresh>",
            "<state>active</state><refresh>60</refresh><refresh>120</refresh>",
            "<state><state>active</state></state>",
            "<state>unknown</state>",
        ] {
            assert_eq!(decode(&xml(body)), Err(()));
        }
        assert_eq!(
            decode(b"<isComposing><state>active</state></isComposing>"),
            Err(())
        );
        assert_eq!(decode(&vec![b' '; 8193]), Err(()));
        assert_eq!(decode(b"<!DOCTYPE isComposing [<!ENTITY signal 'active'>]><isComposing xmlns='urn:ietf:params:xml:ns:im-iscomposing'><state>&signal;</state></isComposing>"), Err(()));
    }
}
