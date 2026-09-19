//! Opt-in Linux-to-phone background clipboard mirroring.
//!
//! Disabled by default. Enable with `HANDOVER_CLIPBOARD_MIRROR=1` or the
//! `clipboard.mirror` IPC method. While enabled, a background task polls the
//! local Wayland clipboard every few seconds and forwards new plain-text
//! content (with HTML and non-file URI representations when offered) to every
//! connected native phone. Images, files, and file URIs never mirror in the
//! background; use the explicit clipboard command for those.
//!
//! Loop prevention is hash-based: content applied locally from a phone, or
//! forwarded by a previous poll, is never sent back.

use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

const POLL_INTERVAL: Duration = Duration::from_secs(3);
const MAX_MIRROR_BYTES: usize = 32 * 1024;

#[derive(Default)]
struct MirrorState {
    /// Hash of the clipboard content last forwarded by the mirror task.
    forwarded: Option<String>,
    /// Hash of the clipboard content last applied locally from a phone.
    remote_applied: Option<String>,
}

fn hash_content(text: &str, html: &Option<String>, uri: &Option<String>) -> String {
    // Length-prefix each part so ("ab","c") and ("a","bc") hash differently.
    let mut key = format!("{}:{text}", text.len());
    if let Some(html) = html {
        key.push_str(&format!("|{}:{html}", html.len()));
    }
    if let Some(uri) = uri {
        key.push_str(&format!("|{}:{uri}", uri.len()));
    }
    // A collision only skips one forward; a cryptographic hash is unnecessary.
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    key.hash(&mut hasher);
    format!("{:016x}", hasher.finish())
}

/// Parse the opt-in flag. Only explicit truthy values enable mirroring;
///
/// everything else, including unset, keeps the default off.
pub(crate) fn enabled_from_env(value: Option<&std::ffi::OsStr>) -> bool {
    matches!(
        value.and_then(|value| value.to_str()),
        Some("1") | Some("true") | Some("on")
    )
}

pub(crate) struct Mirror {
    enabled: Mutex<bool>,
    state: Mutex<MirrorState>,
}

impl Mirror {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            enabled: Mutex::new(enabled),
            state: Mutex::new(MirrorState::default()),
        }
    }

    pub(crate) fn enabled(&self) -> bool {
        *self
            .enabled
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn set_enabled(&self, enabled: bool) {
        *self
            .enabled
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner) = enabled;
    }

    /// Record locally-applied phone content so the next poll skips it.
    pub(crate) fn note_remote(&self, text: &str, html: &Option<String>, uri: &Option<String>) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remote_applied = Some(hash_content(text, html, uri));
    }

    /// Decide whether the currently offered bundle should be forwarded.
    /// Returns the bundle hash when it is new in both directions.
    fn check(&self, text: &str, html: &Option<String>, uri: &Option<String>) -> Option<String> {
        let hash = hash_content(text, html, uri);
        let state = self
            .state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if state.forwarded.as_deref() == Some(&hash)
            || state.remote_applied.as_deref() == Some(&hash)
        {
            return None;
        }
        Some(hash)
    }

    fn note_forwarded(&self, hash: String) {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .forwarded = Some(hash);
    }
}

/// Read the mirrorable text bundle: plain text is required, HTML and
/// non-file URIs ride along, and file URIs disqualify the bundle.
async fn read_mirror_bundle() -> Option<(String, Option<String>, Option<String>)> {
    let offered = crate::clipboard::offered_types().await?;
    if offered.iter().any(|mime| mime.starts_with("image/")) {
        return None;
    }
    let html = if offered.iter().any(|mime| mime == "text/html") {
        crate::clipboard::read_text_mime("text/html").await
    } else {
        None
    };
    let uri = if offered.iter().any(|mime| mime == "text/uri-list") {
        crate::clipboard::read_text_mime("text/uri-list").await
    } else {
        None
    };
    if let Some(uri) = &uri {
        let is_file = uri.lines().any(|line| {
            url::Url::parse(line.trim())
                .ok()
                .and_then(|url| url.to_file_path().ok())
                .is_some()
        });
        if is_file {
            return None;
        }
    }
    let plain = offered
        .iter()
        .find(|mime| *mime == "text/plain;charset=utf-8")
        .or_else(|| offered.iter().find(|mime| *mime == "text/plain"))?;
    let text = crate::clipboard::read_text_mime(plain).await?;
    if text.is_empty()
        || text.len() + html.as_ref().map_or(0, String::len) + uri.as_ref().map_or(0, String::len)
            > MAX_MIRROR_BYTES
    {
        return None;
    }
    Some((text, html, uri))
}

pub(crate) async fn run(mirror: Arc<Mirror>, state: Arc<RwLock<crate::state::StateStore>>) {
    loop {
        tokio::time::sleep(POLL_INTERVAL).await;
        if !mirror.enabled() {
            continue;
        }
        let Some((text, html, uri)) = read_mirror_bundle().await else {
            continue;
        };
        let Some(hash) = mirror.check(&text, &html, &uri) else {
            continue;
        };
        let peers: Vec<String> = state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .snapshot()
            .devices
            .into_iter()
            .filter(|device| {
                device.connected && device.paired && device.id.as_str().starts_with("native:")
            })
            .map(|device| {
                device
                    .id
                    .as_str()
                    .strip_prefix("native:")
                    .unwrap_or_default()
                    .to_owned()
            })
            .collect();
        let mut forwarded = false;
        if let Some(native) = crate::native_backend() {
            for peer in &peers {
                if native
                    .clipboard_set(peer, &text, html.clone(), uri.clone())
                    .is_ok()
                {
                    forwarded = true;
                }
            }
        }
        if forwarded {
            mirror.note_forwarded(hash);
        }
        // When nothing forwards, the hash stays unrecorded so a later poll
        // retries once a phone is reachable. The phone's applied/rejected
        // result arrives as its own device command event.
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn mirror() -> Mirror {
        Mirror::new(true)
    }

    #[test]
    fn new_content_forwards_once() {
        let mirror = mirror();
        let hash = mirror
            .check("hello", &None, &None)
            .expect("new content forwards");
        mirror.note_forwarded(hash);
        assert!(mirror.check("hello", &None, &None).is_none());
    }

    #[test]
    fn remote_applied_content_is_never_sent_back() {
        let mirror = mirror();
        mirror.note_remote("from phone", &None, &None);
        assert!(mirror.check("from phone", &None, &None).is_none());
        // Different local content still forwards.
        assert!(mirror.check("typed locally", &None, &None).is_some());
    }

    #[test]
    fn html_and_uri_parts_join_the_identity() {
        let mirror = mirror();
        let html = Some("<b>x</b>".to_owned());
        assert!(mirror.check("x", &html, &None).is_some());
        assert!(mirror.check("x", &None, &None).is_some());
        mirror.note_remote("x", &html, &None);
        assert!(mirror.check("x", &html, &None).is_none());
        assert!(mirror.check("x", &None, &None).is_some());
    }

    #[test]
    fn only_explicit_truthy_values_opt_in() {
        assert!(enabled_from_env(Some(std::ffi::OsStr::new("1"))));
        assert!(enabled_from_env(Some(std::ffi::OsStr::new("true"))));
        assert!(enabled_from_env(Some(std::ffi::OsStr::new("on"))));
        assert!(!enabled_from_env(None));
        assert!(!enabled_from_env(Some(std::ffi::OsStr::new(""))));
        assert!(!enabled_from_env(Some(std::ffi::OsStr::new("0"))));
        assert!(!enabled_from_env(Some(std::ffi::OsStr::new("yes"))));
    }
}
