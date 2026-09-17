//! Daemon-side helper glue: contract framing, normalization, secret
//! paths, staging, and supervision.
//!
//! License boundary: this crate is MIT. It never links, imports, or copies
//! AGPL sources, generated protobuf, or companion-protocol internals. The
//! Google-specific relay lives in a separate helper process that speaks the
//! coarse JSON contract in [`contract`]; only opaque ids and already-
//! normalized records cross that boundary.

pub mod contract;
pub mod normalize;
pub mod secrets;
pub mod staging;
pub mod supervisor;

pub use contract::{
    HELPER_PROTOCOL, HelperCommand, HelperEvent, MAX_BUNDLE_BYTES, MAX_HELPER_LINE_BYTES,
};
pub use normalize::{NormalizeError, event_ids, normalize_account, normalize_conversation};
pub use supervisor::{HELPER_BINARY, HELPER_ENV, HelperProcess, SpawnError, find_helper};
