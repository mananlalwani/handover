use std::time::Duration;

pub const WIRE_VERSION: u32 = 1;
pub const MAX_FRAME: usize = 64 * 1024;
pub(crate) const MAX_SESSIONS: usize = 16;
pub(crate) const MAX_SESSIONS_PER_SOURCE: usize = 8;
pub(crate) const MAX_ATTEMPTS_PER_SOURCE: usize = 16;
pub(crate) const ATTEMPT_WINDOW: Duration = Duration::from_secs(10);
pub(crate) const MAX_TRACKED_SOURCES: usize = 256;
pub(crate) const PREAUTH_READ_TIMEOUT: Duration = Duration::from_secs(5);
pub(crate) const SESSION_IDLE_TIMEOUT: Duration = Duration::from_secs(300);
pub(crate) const SESSION_PING_INTERVAL: Duration = Duration::from_secs(20);
pub(crate) const LISTEN_PORT: u16 = 24837;
// Bounds for native notification fields. They keep one phone from filling the
// frame budget with a single oversized field and mirror the Android sender's
// truncation limits so both sides pin the same contract.
pub(crate) const MAX_NOTIFICATION_KEY: usize = 256;
pub(crate) const MAX_NOTIFICATION_APP: usize = 128;
pub(crate) const MAX_NOTIFICATION_TITLE: usize = 512;
pub(crate) const MAX_NOTIFICATION_BODY: usize = 4096;
pub(crate) const MAX_NOTIFICATION_ACTIONS: usize = 8;
pub(crate) const MAX_NOTIFICATION_ACTION_ID: usize = 64;
pub(crate) const MAX_NOTIFICATION_ACTION_LABEL: usize = 128;
pub(crate) const MAX_NOTIFICATIONS_PER_SYNC: usize = 64;
pub(crate) const MAX_NOTIFICATION_REPLY: usize = 1024;
// Bounds for native media fields. They mirror the Android sender's truncation
// limits so both sides pin the same contract; volume is never transported.
pub(crate) const MAX_MEDIA_PLAYER: usize = 128;
pub(crate) const MAX_MEDIA_APP: usize = 128;
pub(crate) const MAX_MEDIA_TEXT: usize = 512;
pub(crate) const MAX_MEDIA_SESSIONS_PER_SYNC: usize = 16;
pub(crate) const MAX_MEDIA_POSITION_MS: u64 = i32::MAX as u64;
pub(crate) const MAX_OUTBOX_PER_PEER: usize = 32;
pub(crate) const MAX_SHARE_SIZE: u64 = 100 * 1024 * 1024;
/// Aggregate cap for received shares: one 100 MiB file is legal, but
/// a paired endpoint must not fill the disk by repeating valid sends.
pub(crate) const MAX_RECEIVED_BYTES: u64 = 1024 * 1024 * 1024;
pub(crate) const MAX_RECEIVED_FILES: usize = 1024;
/// Received shares older than this are garbage collected.
pub(crate) const RECEIVED_TTL: Duration = Duration::from_secs(7 * 24 * 60 * 60);
pub(crate) const SHARE_BUFFER: usize = 32 * 1024;
pub(crate) const MAX_PENDING_SHARES_PER_PEER: usize = 32;
pub(crate) const SHARE_RESULT_TIMEOUT: Duration = Duration::from_secs(120);
pub(crate) const INBOUND_TRANSFER_DEADLINE: Duration = Duration::from_secs(120);
