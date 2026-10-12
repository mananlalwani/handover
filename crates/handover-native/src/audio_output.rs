//! Own the temporary phone output and restore app routes when its last user exits.
use std::collections::BTreeMap;
use std::io::{self, BufRead, Read};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(3);
const OUTPUT_LIMIT: u64 = 1024 * 1024;

fn pactl(args: &[&str]) -> io::Result<Vec<u8>> {
    let mut child = Command::new("pactl")
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| io::Error::other("missing pactl output"))?;
    let (sender, receiver) = std::sync::mpsc::channel();
    let reader = std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = stdout
            .take(OUTPUT_LIMIT + 1)
            .read_to_end(&mut bytes)
            .map(|_| bytes);
        let _ = sender.send(result);
    });
    let deadline = Instant::now() + COMMAND_TIMEOUT;
    let result = (|| {
        let bytes = receiver
            .recv_timeout(COMMAND_TIMEOUT)
            .map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "audio command timed out"))??;
        if bytes.len() as u64 > OUTPUT_LIMIT {
            return Err(io::Error::other("audio output too large"));
        }
        loop {
            if let Some(status) = child.try_wait()? {
                return if status.success() {
                    Ok(bytes)
                } else {
                    Err(io::Error::other("audio command rejected"))
                };
            }
            if Instant::now() >= deadline {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "audio command timed out",
                ));
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    })();
    let _ = child.kill();
    let _ = child.wait();
    let _ = reader.join();
    result
}

fn list<T: serde::de::DeserializeOwned>(kind: &str) -> io::Result<Vec<T>> {
    serde_json::from_slice(&pactl(&["--format=json", "list", kind])?).map_err(io::Error::other)
}

#[derive(Deserialize)]
struct Sink {
    index: u32,
    name: String,
    mute: bool,
    owner_module: Option<u32>,
}

#[derive(Deserialize)]
struct Input {
    index: u32,
    sink: u32,
    #[serde(deserialize_with = "client_id")]
    client: Option<u32>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum ClientIndex {
    Number(u32),
    Text(String),
}

fn client_id<'de, D: serde::Deserializer<'de>>(deserializer: D) -> Result<Option<u32>, D::Error> {
    Option::<ClientIndex>::deserialize(deserializer)?
        .map(|index| match index {
            ClientIndex::Number(index) => Ok(index),
            ClientIndex::Text(index) => index
                .parse()
                .map_err(|_| serde::de::Error::custom("invalid audio client index")),
        })
        .transpose()
}

#[derive(Deserialize, Serialize)]
struct SavedInput {
    sink: String,
    client: Option<u32>,
}

#[derive(Deserialize, Serialize)]
struct SavedSink {
    index: u32,
    mute: bool,
}

#[derive(Deserialize, Serialize)]
struct Journal {
    sink: String,
    module: u32,
    default_sink: String,
    muted: BTreeMap<String, SavedSink>,
    inputs: BTreeMap<u32, SavedInput>,
}

struct Route {
    journal: Journal,
    path: PathBuf,
    healthy: Arc<AtomicBool>,
    restored: bool,
}

impl Route {
    fn save(&self) -> io::Result<()> {
        let temporary = self.path.with_extension("tmp");
        let bytes = serde_json::to_vec(&self.journal)?;
        if bytes.len() as u64 > OUTPUT_LIMIT {
            return Err(io::Error::other("audio journal too large"));
        }
        crate::identity::write_private(&temporary, &bytes).map_err(io::Error::other)?;
        std::fs::rename(temporary, &self.path)
    }

    fn route_apps(&mut self) -> io::Result<()> {
        let sinks: Vec<Sink> = list("sinks")?;
        let target = sinks
            .iter()
            .find(|s| s.name == self.journal.sink && s.owner_module == Some(self.journal.module))
            .ok_or_else(|| io::Error::other("phone output disappeared"))?;
        // Physical outputs stay selected for laptop volume/mute keys. Mute
        // them before moving streams, including newly plugged-in outputs.
        for sink in &sinks {
            if sink.index == target.index {
                continue;
            }
            if !self
                .journal
                .muted
                .get(&sink.name)
                .is_some_and(|saved| saved.index == sink.index)
            {
                self.journal.muted.insert(
                    sink.name.clone(),
                    SavedSink {
                        index: sink.index,
                        mute: sink.mute,
                    },
                );
                self.save()?;
            }
            if !sink.mute {
                pactl(&["set-sink-mute", &sink.name, "1"])?;
            }
        }
        let inputs: Vec<Input> = list("sink-inputs")?;
        self.journal.inputs.retain(|index, saved| {
            inputs
                .iter()
                .any(|input| input.index == *index && input.client == saved.client)
        });
        if inputs.len() > 4096 {
            return Err(io::Error::other("too many audio streams"));
        }
        for input in inputs {
            if input.sink == target.index {
                continue;
            }
            let Some(source) = sinks.iter().find(|s| s.index == input.sink) else {
                // A topology event can race these snapshots. The queued
                // sink event reconciles the stream with the next snapshot.
                continue;
            };
            self.journal.inputs.insert(
                input.index,
                SavedInput {
                    sink: source.name.clone(),
                    client: input.client,
                },
            );
            self.save()?;
            if pactl(&[
                "move-sink-input",
                &input.index.to_string(),
                &self.journal.sink,
            ])
            .is_err()
                && list::<Input>("sink-inputs")?
                    .iter()
                    .any(|i| i.index == input.index && i.client == input.client)
            {
                return Err(io::Error::other("could not route app audio"));
            }
        }
        Ok(())
    }

    fn restore(&mut self) -> io::Result<()> {
        self.healthy.store(false, Ordering::Release);
        let sinks: Vec<Sink> = list("sinks")?;
        let target = sinks
            .iter()
            .find(|s| s.name == self.journal.sink && s.owner_module == Some(self.journal.module));
        let mut failed = false;
        if let Some(target) = target {
            for input in list::<Input>("sink-inputs")? {
                if input.sink != target.index {
                    continue;
                }
                let saved = self
                    .journal
                    .inputs
                    .get(&input.index)
                    .filter(|s| s.client == input.client);
                let destination = saved
                    .map(|s| s.sink.as_str())
                    .filter(|name| sinks.iter().any(|s| s.name == *name))
                    .or_else(|| {
                        sinks
                            .iter()
                            .any(|s| s.name == self.journal.default_sink)
                            .then_some(self.journal.default_sink.as_str())
                    });
                if let Some(destination) = destination {
                    failed |=
                        pactl(&["move-sink-input", &input.index.to_string(), destination]).is_err();
                }
            }
        }
        // Restore mute last so restoring an app cannot briefly play twice.
        for (name, saved) in &self.journal.muted {
            if sinks
                .iter()
                .any(|s| s.name == *name && s.index == saved.index)
            {
                failed |=
                    pactl(&["set-sink-mute", name, if saved.mute { "1" } else { "0" }]).is_err();
            }
        }
        if !failed {
            if target.is_some() {
                pactl(&["unload-module", &self.journal.module.to_string()])?;
            }
            match std::fs::remove_file(&self.path) {
                Ok(()) => {}
                Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                Err(error) => return Err(error),
            }
            self.restored = true;
            Ok(())
        } else {
            Err(io::Error::other("could not restore audio routing"))
        }
    }
}

impl Drop for Route {
    fn drop(&mut self) {
        if !self.restored && self.restore().is_err() {
            tracing::warn!("audio routing restoration deferred until next startup");
        }
    }
}

struct Group {
    users: usize,
    route: Arc<Mutex<Route>>,
    watcher: Child,
    worker: Option<std::thread::JoinHandle<()>>,
}

impl Drop for Group {
    fn drop(&mut self) {
        self.route
            .lock()
            .unwrap()
            .healthy
            .store(false, Ordering::Release);
        let _ = self.watcher.kill();
        let _ = self.watcher.wait();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

pub(crate) struct OutputRouter {
    path: PathBuf,
    group: Option<Group>,
    stopping: bool,
}

impl OutputRouter {
    pub(crate) fn new(directory: &std::path::Path) -> Self {
        let mut router = Self {
            path: directory.join("audio-routing.json"),
            group: None,
            stopping: false,
        };
        if router.recover().is_err() {
            tracing::warn!("previous audio routing could not be restored yet");
        }
        router
    }

    fn recover(&mut self) -> io::Result<()> {
        if !self.path.exists() {
            return Ok(());
        }
        let mut bytes = Vec::new();
        std::fs::File::open(&self.path)?
            .take(OUTPUT_LIMIT + 1)
            .read_to_end(&mut bytes)?;
        if bytes.len() as u64 > OUTPUT_LIMIT {
            return Err(io::Error::other("audio journal too large"));
        }
        let journal: Journal = serde_json::from_slice(&bytes)?;
        // The file lives in the identity directory; only our uniquely named
        // sink and exact module ID may be unloaded during recovery.
        if !journal.sink.starts_with("handover_phone_") {
            return Err(io::Error::other("invalid audio journal"));
        }
        let mut route = Route {
            journal,
            path: self.path.clone(),
            healthy: Arc::new(AtomicBool::new(false)),
            restored: false,
        };
        route.restore()
    }

    pub(crate) fn acquire(router: &Arc<Mutex<Self>>) -> io::Result<OutputLease> {
        let mut inner = router.lock().unwrap();
        if inner.stopping {
            return Err(io::Error::other("audio output stopping"));
        }
        if inner.group.is_none() {
            inner.recover()?;
            let mut random = [0u8; 8];
            openssl::rand::rand_bytes(&mut random).map_err(io::Error::other)?;
            let sink = format!("handover_phone_{}", hex::encode(random));
            let default_sink = String::from_utf8(pactl(&["get-default-sink"])?)
                .map_err(io::Error::other)?
                .trim()
                .to_owned();
            let module = String::from_utf8(pactl(&[
                "load-module",
                "module-null-sink",
                &format!("sink_name={sink}"),
                "rate=48000",
                "channels=2",
                "format=s16le",
                "sink_properties=device.description=Handover",
            ])?)
            .map_err(io::Error::other)?
            .trim()
            .parse()
            .map_err(io::Error::other)?;
            let healthy = Arc::new(AtomicBool::new(true));
            let route = Arc::new(Mutex::new(Route {
                journal: Journal {
                    sink,
                    module,
                    default_sink,
                    muted: BTreeMap::new(),
                    inputs: BTreeMap::new(),
                },
                path: inner.path.clone(),
                healthy: healthy.clone(),
                restored: false,
            }));
            route.lock().unwrap().save()?;
            let mut watcher = Command::new("pactl")
                .arg("subscribe")
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()?;
            let Some(stdout) = watcher.stdout.take() else {
                let _ = watcher.kill();
                return Err(io::Error::other("missing audio events"));
            };
            // Subscribe before the first snapshot so new app streams cannot
            // fall into the gap between setup and event observation.
            if let Err(error) = route.lock().unwrap().route_apps() {
                let _ = watcher.kill();
                let _ = watcher.wait();
                return Err(error);
            }
            let worker_route = route.clone();
            let worker = std::thread::spawn(move || {
                let reader = io::BufReader::new(stdout);
                for line in reader.lines() {
                    let Ok(line) = line else { break };
                    if !healthy.load(Ordering::Acquire) || line.len() > 256 {
                        break;
                    }
                    if (line.contains(" on sink-input ") || line.contains(" on sink "))
                        && worker_route.lock().unwrap().route_apps().is_err()
                    {
                        break;
                    }
                }
                healthy.store(false, Ordering::Release);
            });
            inner.group = Some(Group {
                users: 0,
                route,
                watcher,
                worker: Some(worker),
            });
        }
        let group = inner.group.as_mut().unwrap();
        group.users += 1;
        let route = group.route.lock().unwrap();
        Ok(OutputLease {
            router: router.clone(),
            monitor: format!("{}.monitor", route.journal.sink),
            healthy: route.healthy.clone(),
        })
    }

    pub(crate) fn shutdown(&mut self) {
        self.stopping = true;
        self.group.take();
    }
}

pub(crate) struct OutputLease {
    router: Arc<Mutex<OutputRouter>>,
    pub(crate) monitor: String,
    healthy: Arc<AtomicBool>,
}

impl OutputLease {
    pub(crate) fn healthy(&self) -> bool {
        self.healthy.load(Ordering::Acquire)
    }
}

impl Drop for OutputLease {
    fn drop(&mut self) {
        let mut router = self.router.lock().unwrap();
        if let Some(group) = &mut router.group {
            group.users -= 1;
            if group.users == 0 {
                router.group.take();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_client_ids_accept_pulse_and_pipewire_json() {
        for client in [
            serde_json::json!(152),
            serde_json::json!("152"),
            serde_json::Value::Null,
        ] {
            let input: Input =
                serde_json::from_value(serde_json::json!({"index":1,"sink":61,"client":client}))
                    .unwrap();
            assert_eq!(
                input.client,
                if client.is_null() { None } else { Some(152) }
            );
        }
    }

    #[test]
    #[ignore = "requires a live local PulseAudio or PipeWire server"]
    fn phone_output_routes_and_restores() {
        let before: Vec<Sink> = list("sinks").unwrap();
        let default = pactl(&["get-default-sink"]).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let router = Arc::new(Mutex::new(OutputRouter::new(directory.path())));
        let first = OutputRouter::acquire(&router).unwrap();
        let second = OutputRouter::acquire(&router).unwrap();
        assert_eq!(first.monitor, second.monitor);
        assert_eq!(pactl(&["get-default-sink"]).unwrap(), default);
        assert!(
            list::<Sink>("sinks")
                .unwrap()
                .iter()
                .filter(|s| !first.monitor.starts_with(&format!("{}.", s.name)))
                .all(|s| s.mute)
        );
        drop(first);
        assert!(second.healthy());
        drop(second);
        let after: Vec<Sink> = list("sinks").unwrap();
        assert_eq!(after.len(), before.len());
        for original in before {
            assert_eq!(
                after.iter().find(|s| s.name == original.name).unwrap().mute,
                original.mute
            );
        }
        assert!(!directory.path().join("audio-routing.json").exists());
    }
}
