//! Durable send metadata. Journal entries never contain payloads and never replay.
use std::io::{self, Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

use handover_core::{
    MessageStatus, MessagingEvent, OutgoingOperation, OutgoingOutcome, StateEvent,
};
use serde::{Deserialize, Serialize};
use tokio::sync::broadcast;

use crate::state::StateStore;

const MAX_BYTES: u64 = 1024 * 1024;
const MAX_OPERATIONS: usize = 512;
static READABLE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

#[derive(Serialize, Deserialize)]
struct Journal {
    version: u32,
    operations: Vec<OutgoingOperation>,
}

pub(crate) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

pub(crate) fn new_id() -> io::Result<String> {
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(format!(
        "send-{}",
        bytes
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect::<String>()
    ))
}

#[cfg(test)]
fn path() -> io::Result<PathBuf> {
    // Backend unit tests also publish events. Never let their asynchronous
    // journal writes reach the user's state directory or race XDG overrides.
    static DIRECTORY: std::sync::OnceLock<tempfile::TempDir> = std::sync::OnceLock::new();
    Ok(DIRECTORY
        .get_or_init(|| tempfile::tempdir().expect("test journal directory"))
        .path()
        .join("outgoing-operations.json"))
}

#[cfg(not(test))]
fn path() -> io::Result<PathBuf> {
    let base = std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "state directory unavailable"))?;
    Ok(base.join("handover/outgoing-operations.json"))
}

pub(crate) fn restore(state: &mut StateStore) -> io::Result<()> {
    let result = path().and_then(|path| restore_at(state, &path));
    READABLE.store(result.is_ok(), std::sync::atomic::Ordering::Relaxed);
    result
}

fn restore_at(state: &mut StateStore, path: &Path) -> io::Result<()> {
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    let metadata = file.metadata()?;
    if !metadata.is_file() || metadata.len() > MAX_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "outgoing journal exceeds bounds",
        ));
    }
    let mut bytes = Vec::new();
    file.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "outgoing journal exceeds bounds",
        ));
    }
    let journal: Journal = serde_json::from_slice(&bytes)?;
    if journal.version != 1 || journal.operations.len() > MAX_OPERATIONS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "outgoing journal is incompatible",
        ));
    }
    let mut ids = std::collections::HashSet::new();
    for operation in &journal.operations {
        if handover_core::messaging::validate_outgoing(operation).is_err()
            || !ids.insert(&operation.id)
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "outgoing journal contains invalid records",
            ));
        }
    }
    for mut operation in journal.operations {
        if matches!(
            operation.outcome,
            OutgoingOutcome::Submitting | OutgoingOutcome::Provider(MessageStatus::Accepted)
        ) {
            operation.outcome = OutgoingOutcome::Unknown;
            operation.updated_at = now().max(operation.updated_at);
        }
        state.apply(StateEvent::Messaging(MessagingEvent::Outgoing(operation)));
    }
    Ok(())
}

fn write_at(path: &Path, operations: Vec<OutgoingOperation>) -> io::Result<()> {
    if operations.len() > MAX_OPERATIONS {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "outgoing journal exceeds bounds",
        ));
    }
    let encoded = serde_json::to_vec(&Journal {
        version: 1,
        operations,
    })?;
    if encoded.len() as u64 > MAX_BYTES {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "outgoing journal exceeds bounds",
        ));
    }
    let directory = path
        .parent()
        .ok_or_else(|| io::Error::other("missing journal directory"))?;
    std::fs::create_dir_all(directory)?;
    std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    temporary.as_file_mut().write_all(&encoded)?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    std::fs::File::open(directory)?.sync_all()?;
    Ok(())
}

pub(crate) async fn persist(state: &Arc<RwLock<StateStore>>) -> io::Result<()> {
    if !READABLE.load(std::sync::atomic::Ordering::Relaxed) {
        return Err(io::Error::other(
            "outgoing journal could not be restored; preserving existing file",
        ));
    }
    // Snapshot under the publish lock, so an older write cannot replace newer state.
    static PUBLISH: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = PUBLISH.lock().await;
    let operations = state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .messaging()
        .snapshot_outgoing();
    let path = path()?;
    tokio::task::spawn_blocking(move || write_at(&path, operations))
        .await
        .map_err(io::Error::other)?
}

pub(crate) fn schedule_persist(state: &Arc<RwLock<StateStore>>) {
    use std::sync::atomic::{AtomicBool, Ordering};
    static DIRTY: AtomicBool = AtomicBool::new(false);
    static SCHEDULED: AtomicBool = AtomicBool::new(false);
    DIRTY.store(true, Ordering::SeqCst);
    if SCHEDULED.swap(true, Ordering::SeqCst) {
        return;
    }
    if let Ok(runtime) = tokio::runtime::Handle::try_current() {
        let state = Arc::clone(state);
        runtime.spawn(async move {
            loop {
                DIRTY.store(false, Ordering::SeqCst);
                if let Err(error) = persist(&state).await {
                    tracing::warn!(%error, "could not persist outgoing operations");
                }
                SCHEDULED.store(false, Ordering::SeqCst);
                if !DIRTY.load(Ordering::SeqCst) || SCHEDULED.swap(true, Ordering::SeqCst) {
                    break;
                }
            }
        });
    } else {
        SCHEDULED.store(false, Ordering::SeqCst);
    }
}

pub(crate) async fn begin(
    state: &Arc<RwLock<StateStore>>,
    events: &broadcast::Sender<StateEvent>,
    conversation_id: handover_core::ConversationId,
    kind: handover_core::OutgoingOperationKind,
) -> io::Result<String> {
    static BEGIN: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
    let _guard = BEGIN.lock().await;
    let records = state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .messaging()
        .snapshot_outgoing();
    if records.len() >= MAX_OPERATIONS {
        let oldest = records
            .iter()
            .filter(|record| {
                !matches!(
                    record.outcome,
                    OutgoingOutcome::Submitting
                        | OutgoingOutcome::Provider(MessageStatus::Accepted)
                )
            })
            .min_by_key(|record| (record.created_at, &record.id))
            .ok_or_else(|| io::Error::other("outgoing operation capacity exhausted"))?;
        crate::apply_backend_event(
            state,
            events,
            StateEvent::Messaging(MessagingEvent::OutgoingRemoved(oldest.id.clone())),
        );
    }
    let id = new_id()?;
    let created_at = now();
    let operation = OutgoingOperation {
        id: id.clone(),
        conversation_id,
        kind,
        created_at,
        updated_at: created_at,
        outcome: OutgoingOutcome::Submitting,
        message_id: None,
    };
    crate::apply_backend_event(
        state,
        events,
        StateEvent::Messaging(MessagingEvent::Outgoing(operation)),
    );
    if let Err(error) = persist(state).await {
        crate::apply_backend_event(
            state,
            events,
            StateEvent::Messaging(MessagingEvent::OutgoingRemoved(id)),
        );
        return Err(error);
    }
    Ok(id)
}

pub(crate) fn update(
    state: &Arc<RwLock<StateStore>>,
    events: &broadcast::Sender<StateEvent>,
    id: &str,
    outcome: OutgoingOutcome,
    message_id: Option<handover_core::MessageId>,
) {
    let operation = state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .messaging()
        .outgoing(id)
        .cloned();
    if let Some(mut operation) = operation {
        operation.outcome = outcome;
        operation.updated_at = now().max(operation.updated_at);
        if message_id.is_some() {
            operation.message_id = message_id;
        }
        crate::apply_backend_event(
            state,
            events,
            StateEvent::Messaging(MessagingEvent::Outgoing(operation)),
        );
    }
}

pub(crate) fn mark_unknown(
    state: &Arc<RwLock<StateStore>>,
    events: &broadcast::Sender<StateEvent>,
) {
    let operations = state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .messaging()
        .snapshot_outgoing();
    for operation in operations {
        if matches!(
            operation.outcome,
            OutgoingOutcome::Submitting | OutgoingOutcome::Provider(MessageStatus::Accepted)
        ) {
            update(state, events, &operation.id, OutgoingOutcome::Unknown, None);
        }
    }
}

pub(crate) async fn run_expiry(
    state: Arc<RwLock<StateStore>>,
    events: broadcast::Sender<StateEvent>,
) {
    let mut receiver = events.subscribe();
    loop {
        let records = state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .messaging()
            .snapshot_outgoing();
        let deadline = records
            .iter()
            .filter(|operation| {
                matches!(
                    operation.outcome,
                    OutgoingOutcome::Submitting
                        | OutgoingOutcome::Provider(MessageStatus::Accepted)
                )
            })
            .map(|operation| operation.created_at.saturating_add(10 * 60))
            .min();
        if let Some(deadline) = deadline {
            if deadline <= now() {
                for operation in records {
                    if operation.created_at.saturating_add(10 * 60) <= now()
                        && matches!(
                            operation.outcome,
                            OutgoingOutcome::Submitting
                                | OutgoingOutcome::Provider(MessageStatus::Accepted)
                        )
                    {
                        update(
                            &state,
                            &events,
                            &operation.id,
                            OutgoingOutcome::Unknown,
                            None,
                        );
                    }
                }
                continue;
            }
            tokio::select! {
                _ = tokio::time::sleep(std::time::Duration::from_secs(deadline.saturating_sub(now()))) => {}
                result = receiver.recv() => {
                    if matches!(result, Err(broadcast::error::RecvError::Closed)) { return; }
                }
            }
        } else if matches!(
            receiver.recv().await,
            Err(broadcast::error::RecvError::Closed)
        ) {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use handover_core::{ConversationId, MessagingAccountId, OutgoingOperationKind};

    fn operation(id: &str, outcome: OutgoingOutcome) -> OutgoingOperation {
        OutgoingOperation {
            id: id.into(),
            conversation_id: ConversationId::new(
                MessagingAccountId::new("gmessages:test"),
                "thread",
            ),
            kind: OutgoingOperationKind::Text,
            created_at: 1,
            updated_at: 1,
            outcome,
            message_id: None,
        }
    }

    #[test]
    fn local_acceptance_cannot_erase_unknown_send_outcome() {
        let state = Arc::new(RwLock::new(StateStore::default()));
        let (events, _) = broadcast::channel(8);
        crate::apply_backend_event(
            &state,
            &events,
            StateEvent::Messaging(MessagingEvent::Outgoing(operation(
                "send",
                OutgoingOutcome::Unknown,
            ))),
        );
        update(
            &state,
            &events,
            "send",
            OutgoingOutcome::Provider(MessageStatus::Accepted),
            None,
        );
        assert_eq!(
            state
                .read()
                .unwrap()
                .messaging()
                .outgoing("send")
                .unwrap()
                .outcome,
            OutgoingOutcome::Unknown
        );
        let message = handover_core::MessageId::new(
            operation("send", OutgoingOutcome::Unknown).conversation_id,
            "phone-message",
        );
        update(
            &state,
            &events,
            "send",
            OutgoingOutcome::Provider(MessageStatus::Accepted),
            Some(message.clone()),
        );
        let guard = state.read().unwrap();
        let restored = guard.messaging().outgoing("send").unwrap();
        assert_eq!(
            restored.outcome,
            OutgoingOutcome::Provider(MessageStatus::Accepted)
        );
        assert_eq!(restored.message_id, Some(message));
    }

    #[tokio::test]
    async fn late_delivery_resolves_a_journal_restored_operation_and_survives_another_restart() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("journal.json");
        let id = "send-0123456789abcdef0123456789abcdef";
        write_at(
            &path,
            vec![operation(
                id,
                OutgoingOutcome::Provider(MessageStatus::Accepted),
            )],
        )
        .unwrap();
        let mut restored = StateStore::default();
        restore_at(&mut restored, &path).unwrap();
        let pending = restored.messaging().outgoing(id).unwrap();
        assert_eq!(pending.outcome, OutgoingOutcome::Unknown);
        assert!(pending.message_id.is_none());
        let message =
            handover_core::MessageId::new(pending.conversation_id.clone(), "phone-message");
        let state = Arc::new(RwLock::new(restored));
        let (events, _) = broadcast::channel(8);
        update(
            &state,
            &events,
            id,
            OutgoingOutcome::Provider(MessageStatus::Delivered),
            Some(message.clone()),
        );
        // A late local acknowledgement must not erase the phone evidence.
        update(
            &state,
            &events,
            id,
            OutgoingOutcome::Provider(MessageStatus::Accepted),
            None,
        );
        write_at(&path, state.read().unwrap().messaging().snapshot_outgoing()).unwrap();
        let mut second_restart = StateStore::default();
        restore_at(&mut second_restart, &path).unwrap();
        let recovered = second_restart.messaging().outgoing(id).unwrap();
        assert_eq!(
            recovered.outcome,
            OutgoingOutcome::Provider(MessageStatus::Delivered)
        );
        assert_eq!(recovered.message_id, Some(message));
        assert_eq!(second_restart.messaging().snapshot_outgoing().len(), 1);
    }
    #[test]
    fn restart_restores_uncertainty_without_changing_delivery_evidence() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("journal.json");
        write_at(
            &path,
            vec![
                operation("submitting", OutgoingOutcome::Submitting),
                operation(
                    "accepted",
                    OutgoingOutcome::Provider(MessageStatus::Accepted),
                ),
                operation(
                    "delivered",
                    OutgoingOutcome::Provider(MessageStatus::Delivered),
                ),
            ],
        )
        .unwrap();
        let mut state = StateStore::default();
        restore_at(&mut state, &path).unwrap();
        assert_eq!(
            state.messaging().outgoing("submitting").unwrap().outcome,
            OutgoingOutcome::Unknown
        );
        assert_eq!(
            state.messaging().outgoing("accepted").unwrap().outcome,
            OutgoingOutcome::Unknown
        );
        assert_eq!(
            state.messaging().outgoing("delivered").unwrap().outcome,
            OutgoingOutcome::Provider(MessageStatus::Delivered)
        );
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(
            std::fs::metadata(directory.path())
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700
        );
    }

    #[test]
    fn late_evidence_resolves_unknown_and_acknowledgement_cannot_rewind_it() {
        let mut state = StateStore::default();
        let mut record = operation("send", OutgoingOutcome::Unknown);
        state.apply(StateEvent::Messaging(MessagingEvent::Outgoing(
            record.clone(),
        )));
        record.outcome = OutgoingOutcome::Provider(MessageStatus::Delivered);
        state.apply(StateEvent::Messaging(MessagingEvent::Outgoing(
            record.clone(),
        )));
        record.outcome = OutgoingOutcome::Provider(MessageStatus::Accepted);
        assert!(
            !state
                .apply(StateEvent::Messaging(MessagingEvent::Outgoing(record)))
                .changed
        );
        assert_eq!(
            state.messaging().outgoing("send").unwrap().outcome,
            OutgoingOutcome::Provider(MessageStatus::Delivered)
        );
        state.apply(StateEvent::Messaging(MessagingEvent::Account(
            handover_core::MessagingAccountEvent::Removed(MessagingAccountId::new(
                "gmessages:test",
            )),
        )));
        assert!(state.messaging().snapshot_outgoing().is_empty());
    }

    #[test]
    fn oversized_journal_is_rejected_without_loading_partial_state() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("journal.json");
        std::fs::write(
            &path,
            serde_json::to_vec(&Journal {
                version: 1,
                operations: (0..513)
                    .map(|index| operation(&index.to_string(), OutgoingOutcome::Unknown))
                    .collect(),
            })
            .unwrap(),
        )
        .unwrap();
        let mut state = StateStore::default();
        assert!(restore_at(&mut state, &path).is_err());
        assert!(state.messaging().snapshot_outgoing().is_empty());
    }

    #[tokio::test]
    async fn expiry_marks_unresolved_operations_unknown_without_replay() {
        let mut store = StateStore::default();
        store.apply(StateEvent::Messaging(MessagingEvent::Outgoing(operation(
            "expired",
            OutgoingOutcome::Provider(MessageStatus::Accepted),
        ))));
        let state = Arc::new(RwLock::new(store));
        let (events, mut receiver) = broadcast::channel(8);
        let worker = tokio::spawn(run_expiry(Arc::clone(&state), events));
        let event = tokio::time::timeout(std::time::Duration::from_secs(1), receiver.recv())
            .await
            .unwrap()
            .unwrap();
        assert!(
            matches!(event, StateEvent::Messaging(MessagingEvent::Outgoing(operation))
            if operation.id == "expired" && operation.outcome == OutgoingOutcome::Unknown)
        );
        worker.abort();
    }
}
