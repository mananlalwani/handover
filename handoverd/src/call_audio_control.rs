//! Daemon-owned call routing. Arming is explicit; capture exists only during a
//! reported call and belongs to child processes that die with this daemon.
use crate::{call_audio_backend as backend, state::StateStore};
use handover_core::{
    CallAudioControls, CallAudioRoute, CallAudioRouteState, CallAudioSelection, CallPhase,
    DeviceId, StateEvent,
};
use std::{
    path::PathBuf,
    sync::{
        Arc, OnceLock, RwLock,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    sync::{broadcast, mpsc, oneshot},
};

type Response = oneshot::Sender<Result<CallAudioControls, String>>;
enum Command {
    Get(Response),
    Configure(u64, Option<CallAudioSelection>, Response),
    Wake,
    Shutdown(oneshot::Sender<()>),
}
static COMMANDS: OnceLock<mpsc::Sender<Command>> = OnceLock::new();
static WAKE_PENDING: AtomicBool = AtomicBool::new(false);

pub(crate) fn notify() {
    if let Some(tx) = COMMANDS.get()
        && !WAKE_PENDING.swap(true, Ordering::SeqCst)
        && tx.try_send(Command::Wake).is_err()
    {
        WAKE_PENDING.store(false, Ordering::SeqCst);
    }
}
pub(crate) async fn controls() -> Result<CallAudioControls, String> {
    let (tx, rx) = oneshot::channel();
    COMMANDS
        .get()
        .ok_or("Call audio is unavailable")?
        .try_send(Command::Get(tx))
        .map_err(|_| "Call audio is busy")?;
    rx.await.map_err(|_| "Call audio stopped".to_string())?
}
pub(crate) async fn configure(
    revision: u64,
    selection: Option<CallAudioSelection>,
) -> Result<CallAudioControls, String> {
    let (tx, rx) = oneshot::channel();
    COMMANDS
        .get()
        .ok_or("Call audio is unavailable")?
        .try_send(Command::Configure(revision, selection, tx))
        .map_err(|_| "Call audio is busy")?;
    rx.await.map_err(|_| "Call audio stopped".to_string())?
}
pub(crate) async fn shutdown() {
    if let Some(tx) = COMMANDS.get() {
        let (done, wait) = oneshot::channel();
        if tx.send(Command::Shutdown(done)).await.is_ok() {
            let _ = tokio::time::timeout(Duration::from_secs(10), wait).await;
        }
    }
}
fn journal_path() -> Option<PathBuf> {
    handover_native::NativeBackend::default_directory()
        .ok()
        .map(|p| p.join("call-audio-profile.json"))
}
fn write_profile(path: &std::path::Path, profile: &backend::ProfileRecovery) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or("Call audio profile storage unavailable")?;
    std::fs::create_dir_all(parent).map_err(|_| "Call audio profile storage unavailable")?;
    let mut temp = tempfile::NamedTempFile::new_in(parent)
        .map_err(|_| "Call audio profile storage unavailable")?;
    serde_json::to_writer(temp.as_file_mut(), profile)
        .map_err(|_| "Call audio profile could not be saved")?;
    temp.as_file()
        .sync_all()
        .map_err(|_| "Call audio profile could not be saved")?;
    temp.persist(path)
        .map_err(|_| "Call audio profile could not be saved")?;
    Ok(())
}
fn read_profile(path: &std::path::Path) -> Option<backend::ProfileRecovery> {
    use std::io::Read;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .ok()?
        .take(16 * 1024 + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() <= 16 * 1024)
        .then(|| serde_json::from_slice(&bytes).ok())
        .flatten()
}
fn call_generation(state: &Arc<RwLock<StateStore>>, id: &DeviceId) -> Option<u64> {
    let state = state
        .read()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !state
        .get_device(id)
        .is_some_and(|d| d.connected && d.paired)
    {
        return None;
    }
    state
        .get_call(id)
        .filter(|c| c.phase == CallPhase::OffHook)
        .map(|c| c.generation)
}
struct Controller {
    view: CallAudioControls,
    selection: Option<CallAudioSelection>,
    owned: Option<backend::OwnedRoute>,
    inventory: Option<backend::Inventory>,
    journal: Option<PathBuf>,
    blocked: bool,
    recovery: Option<backend::ProfileRecovery>,
}
impl Controller {
    async fn stop(&mut self) -> Result<(), String> {
        if let Some(profile) = &self.recovery {
            backend::restore_profile(profile).await?;
            self.recovery = None;
        }

        if let Some(mut owned) = self.owned.take()
            && let Err(error) = backend::stop_route(&mut owned).await
        {
            self.owned = Some(owned);
            return Err(error);
        }
        if let Some(path) = &self.journal {
            let _ = std::fs::remove_file(path);
        }
        Ok(())
    }
    async fn refresh(&mut self, state: &Arc<RwLock<StateStore>>) {
        // End microphone capture before waiting for a host inventory refresh.
        if self.owned.is_some()
            && self
                .selection
                .as_ref()
                .is_none_or(|s| call_generation(state, &s.device_id).is_none())
        {
            let _ = self.stop().await;
        }
        if let Some(profile) = &self.recovery {
            if backend::restore_profile(profile).await.is_err() {
                self.view.observed = false;
                self.view.notice=Some("The previous Bluetooth profile could not be restored. Refresh audio devices to try again.".into());
                return;
            }
            self.recovery = None;
            if let Some(path) = &self.journal {
                let _ = std::fs::remove_file(path);
            }
        }
        let inventory = tokio::time::timeout(Duration::from_secs(5), backend::inventory()).await;
        match inventory {
            Ok(Ok(inventory)) => {
                self.view.observed = true;
                self.view.endpoints = inventory.endpoints.clone();
                self.view.inputs = inventory.inputs.clone();
                self.view.outputs = inventory.outputs.clone();
                self.inventory = Some(inventory);
                self.view.notice = None;
            }
            _ => {
                self.view.observed = false;
                self.view.endpoints.clear();
                self.view.inputs.clear();
                self.view.outputs.clear();
                self.inventory = None;
                self.view.notice = Some("PipeWire call-audio devices are unavailable".into());
            }
        }
        let Some(selection) = self.selection.clone() else {
            if self.owned.is_none() {
                self.view.route = None;
            }
            return;
        };
        if state
            .read()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .get_device(&selection.device_id)
            .is_none_or(|d| !d.paired)
        {
            self.selection = None;
            let stopped = self.stop().await;
            self.view.route = None;
            self.view.notice = stopped.err();
            return;
        }
        let generation = call_generation(state, &selection.device_id);
        let available = self
            .view
            .endpoints
            .iter()
            .any(|c| c.id == selection.endpoint_id)
            && self.view.inputs.iter().any(|c| c.id == selection.input_id)
            && self
                .view
                .outputs
                .iter()
                .any(|c| c.id == selection.output_id);
        let mut route = CallAudioRoute {
            device_id: selection.device_id.clone(),
            endpoint_id: selection.endpoint_id.clone(),
            input_id: selection.input_id.clone(),
            output_id: selection.output_id.clone(),
            state: CallAudioRouteState::WaitingForCall,
        };
        if generation.is_none() || !available {
            if let Err(message) = self.stop().await {
                route.state = CallAudioRouteState::Failed { message };
            } else if !available {
                route.state = CallAudioRouteState::Failed {
                    message:
                        "The selected Bluetooth phone, microphone, or speakers are unavailable"
                            .into(),
                };
            }
            if generation.is_none() {
                self.blocked = false;
            }
            self.view.route = Some(route);
            return;
        }
        if self.owned.is_none() && !self.blocked {
            let phone = selection.device_id.clone();
            let state = state.clone();
            let active = move || call_generation(&state, &phone).is_some();
            let journal = self.journal.clone();
            let persist = move |profile: &backend::ProfileRecovery| {
                write_profile(
                    journal
                        .as_deref()
                        .ok_or("Call audio profile storage unavailable")?,
                    profile,
                )
            };
            match backend::start_route(
                self.inventory.as_ref().unwrap(),
                &selection,
                &active,
                &persist,
            )
            .await
            {
                Ok(owned) => self.owned = Some(owned),
                Err(mut message) => {
                    // Setup may have changed the card profile before failing. Recover it
                    // now, and retain its journal if restoration needs another attempt.
                    self.recovery = self.journal.as_deref().and_then(read_profile);
                    if let Err(error) = self.stop().await {
                        message.push_str(". ");
                        message.push_str(&error);
                    }
                    self.blocked = true;
                    route.state = CallAudioRouteState::Failed { message };
                    self.view.route = Some(route);
                    return;
                }
            }
        }
        if let Some(owned) = &mut self.owned {
            if !backend::processes_alive(owned).await.unwrap_or(false) {
                let _ = self.stop().await;
                self.blocked = true;
                route.state = CallAudioRouteState::Failed {
                    message: "The audio route stopped. Choose Use laptop for calls to try again."
                        .into(),
                };
            } else {
                let running = backend::route_present(owned).await.unwrap_or(false);
                route.state = CallAudioRouteState::Routing {
                    duplex_running: running,
                };
            }
        } else if self.blocked {
            route.state = CallAudioRouteState::Failed {
                message: "The audio route needs to be enabled again".into(),
            };
        }
        // The call can end while local setup awaits an audio-server response.
        if call_generation(state, &selection.device_id).is_none() {
            let _ = self.stop().await;
            route.state = CallAudioRouteState::WaitingForCall;
        }
        self.view.route = Some(route);
    }
    fn publish(
        &mut self,
        before: &CallAudioControls,
        state: &Arc<RwLock<StateStore>>,
        events: &broadcast::Sender<StateEvent>,
    ) {
        if &self.view != before {
            self.view.revision = before.revision.saturating_add(1);
            crate::apply_backend_event(
                state,
                events,
                StateEvent::CallAudioControls(self.view.clone()),
            );
        }
    }
}
pub(crate) fn start(state: Arc<RwLock<StateStore>>, events: broadcast::Sender<StateEvent>) {
    let (tx, mut rx) = mpsc::channel(16);
    if COMMANDS.set(tx).is_err() {
        return;
    }
    tokio::spawn(observe_audio());
    tokio::spawn(async move {
        let mut control = Controller {
            view: Default::default(),
            selection: None,
            owned: None,
            inventory: None,
            journal: journal_path(),
            blocked: false,
            recovery: None,
        };
        if let Some(path) = &control.journal {
            let stored = read_profile(path);
            if let Some(profile) = stored {
                if backend::restore_profile(&profile).await.is_ok() {
                    let _ = std::fs::remove_file(path);
                } else {
                    control.recovery = Some(profile);
                }
            }
        }
        let before = control.view.clone();
        control.refresh(&state).await;
        control.publish(&before, &state, &events);
        while let Some(command) = rx.recv().await {
            let before = control.view.clone();
            match command {
                Command::Wake => {
                    WAKE_PENDING.store(false, Ordering::SeqCst);
                    control.refresh(&state).await;
                    control.publish(&before, &state, &events);
                }
                Command::Get(reply) => {
                    control.refresh(&state).await;
                    control.publish(&before, &state, &events);
                    let _ = reply.send(Ok(control.view.clone()));
                }
                Command::Configure(revision, selection, reply) => {
                    let valid = revision == control.view.revision
                        && selection.as_ref().is_none_or(|s| {
                            s.expected_revision == revision
                                && control.view.endpoints.iter().any(|c| c.id == s.endpoint_id)
                                && control.view.inputs.iter().any(|c| c.id == s.input_id)
                                && control.view.outputs.iter().any(|c| c.id == s.output_id)
                                && state
                                    .read()
                                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                                    .get_device(&s.device_id)
                                    .is_some_and(|d| d.connected && d.paired)
                        });
                    if !valid {
                        let _=reply.send(Err("Audio devices or call-audio settings changed. Refresh and choose again.".into()));
                        continue;
                    }
                    control.selection = None;
                    control.blocked = false;
                    if let Err(error) = control.stop().await {
                        control.view.notice = Some(error.clone());
                        control.publish(&before, &state, &events);
                        let _ = reply.send(Err(error));
                        continue;
                    }
                    control.selection = selection;
                    control.refresh(&state).await;
                    control.publish(&before, &state, &events);
                    let _ = reply.send(Ok(control.view.clone()));
                }
                Command::Shutdown(reply) => {
                    control.selection = None;
                    let _ = control.stop().await;
                    let _ = reply.send(());
                    break;
                }
            }
        }
    });
}
async fn observe_audio() {
    loop {
        let child = tokio::process::Command::new("pactl")
            .arg("subscribe")
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .stdin(std::process::Stdio::null())
            .kill_on_drop(true)
            .spawn();
        if let Ok(mut child) = child {
            if let Some(stdout) = child.stdout.take() {
                let mut lines = BufReader::new(stdout.take(1024 * 1024)).lines();
                while let Ok(Some(line)) = lines.next_line().await {
                    if line.contains("card")
                        || line.contains("source")
                        || line.contains("sink")
                        || line.contains("server")
                    {
                        notify();
                    }
                }
            }
            let _ = child.kill().await;
            let _ = child.wait().await;
        }
        notify();
        tokio::time::sleep(Duration::from_secs(10)).await;
    }
}
