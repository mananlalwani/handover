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

use handover_core::{
    AudioApplication, AudioOutputMode, AudioRoutingSettings, AudioSourceSelection,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

const COMMAND_TIMEOUT: Duration = Duration::from_secs(3);
const OUTPUT_LIMIT: u64 = 1024 * 1024;

pub(crate) fn run_command(program: &str, args: &[&str]) -> io::Result<Vec<u8>> {
    let mut child = Command::new(program)
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

fn pactl(args: &[&str]) -> io::Result<Vec<u8>> {
    run_command("pactl", args)
}
trait AudioServer: Send + Sync {
    fn run(&self, args: &[&str]) -> io::Result<Vec<u8>>;
}
struct PulseServer;
impl AudioServer for PulseServer {
    fn run(&self, args: &[&str]) -> io::Result<Vec<u8>> {
        pactl(args)
    }
}
fn server_list<T: serde::de::DeserializeOwned>(
    server: &dyn AudioServer,
    kind: &str,
) -> io::Result<Vec<T>> {
    serde_json::from_slice(&server.run(&["--format=json", "list", kind])?).map_err(io::Error::other)
}

#[cfg(test)]
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
    #[serde(default)]
    owner_module: Option<u32>,
    #[serde(default)]
    properties: BTreeMap<String, serde_json::Value>,
}
impl Input {
    fn application(&self) -> AudioApplication {
        let property = |key: &str| {
            self.properties
                .get(key)
                .and_then(|value| value.as_str())
                .filter(|value| !value.is_empty())
        };
        let identity = property("application.id")
            .or_else(|| property("application.process.binary"))
            .or_else(|| property("application.name"))
            .map(str::to_owned)
            .unwrap_or_else(|| format!("anonymous:{:?}:{}", self.client, self.index));
        let name = property("application.name")
            .or_else(|| property("application.process.binary"))
            .unwrap_or("Audio application")
            .chars()
            .take(160)
            .collect();
        AudioApplication {
            id: format!("app-{}", hex::encode(Sha256::digest(identity.as_bytes()))),
            name,
            available: true,
        }
    }
    fn is_mirror(&self, mirror: Option<&SavedMirror>) -> bool {
        self.properties
            .get("application.id")
            .and_then(|value| value.as_str())
            == Some("org.handover.audio-mirror")
            || mirror.is_some_and(|mirror| self.owner_module == Some(mirror.module))
    }
    fn matches(&self, saved: &SavedInput) -> bool {
        self.client == saved.client
            && saved
                .application
                .as_ref()
                .is_none_or(|id| self.application().id == *id)
    }
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
    #[serde(default)]
    application: Option<String>,
}

#[derive(Deserialize, Serialize)]
struct SavedSink {
    index: u32,
    mute: bool,
}

#[derive(Deserialize, Serialize)]
struct SavedMirror {
    module: u32,
    sink: String,
}
#[derive(Deserialize)]
struct Module {
    index: u32,
    name: String,
    argument: String,
}

#[derive(Deserialize, Serialize)]
struct Journal {
    sink: String,
    module: u32,
    default_sink: String,
    muted: BTreeMap<String, SavedSink>,
    inputs: BTreeMap<u32, SavedInput>,
    #[serde(default)]
    mirror: Option<SavedMirror>,
}

struct Route {
    journal: Journal,
    settings: AudioRoutingSettings,
    server: Arc<dyn AudioServer>,
    path: PathBuf,
    healthy: Arc<AtomicBool>,
    restored: bool,
}

impl Route {
    fn run(&self, args: &[&str]) -> io::Result<Vec<u8>> {
        self.server.run(args)
    }
    fn list<T: serde::de::DeserializeOwned>(&self, kind: &str) -> io::Result<Vec<T>> {
        server_list(self.server.as_ref(), kind)
    }
    fn save(&self) -> io::Result<()> {
        let temporary = self.path.with_extension("tmp");
        let bytes = serde_json::to_vec(&self.journal)?;
        if bytes.len() as u64 > OUTPUT_LIMIT {
            return Err(io::Error::other("audio journal too large"));
        }
        crate::identity::write_private(&temporary, &bytes).map_err(io::Error::other)?;
        std::fs::rename(temporary, &self.path)
    }

    fn restore_mutes(&mut self, sinks: &[Sink]) -> io::Result<()> {
        let names: Vec<_> = self.journal.muted.keys().cloned().collect();
        for name in names {
            let saved = &self.journal.muted[&name];
            if sinks
                .iter()
                .any(|sink| sink.name == name && sink.index == saved.index)
            {
                self.run(&["set-sink-mute", &name, if saved.mute { "1" } else { "0" }])?;
            }
            self.journal.muted.remove(&name);
            self.save()?;
        }
        Ok(())
    }
    fn remove_mirror(&mut self) -> io::Result<()> {
        if let Some(saved) = &self.journal.mirror {
            let source = format!("source={}.monitor", self.journal.sink);
            let modules: Vec<Module> = self.list("modules")?;
            if modules.iter().any(|module| {
                module.index == saved.module
                    && module.name == "module-loopback"
                    && module
                        .argument
                        .split_whitespace()
                        .any(|argument| argument == format!("sink={}", saved.sink))
                    && module
                        .argument
                        .split_whitespace()
                        .any(|argument| argument == source)
            }) {
                self.run(&["unload-module", &saved.module.to_string()])?;
            }
            self.journal.mirror = None;
            self.save()?;
        }
        Ok(())
    }
    fn reconcile_mirror(&mut self, sinks: &[Sink]) -> io::Result<()> {
        if self.settings.output != AudioOutputMode::Both {
            return self.remove_mirror();
        }
        let default = String::from_utf8(self.run(&["get-default-sink"])?)
            .map_err(io::Error::other)?
            .trim()
            .to_owned();
        let destination = sinks
            .iter()
            .find(|sink| sink.name == default && sink.name != self.journal.sink)
            .or_else(|| {
                sinks.iter().find(|sink| {
                    sink.name == self.journal.default_sink && sink.name != self.journal.sink
                })
            })
            .ok_or_else(|| io::Error::other("no laptop output available"))?;
        if destination
            .name
            .bytes()
            .any(|byte| byte.is_ascii_whitespace() || b"\"'\\".contains(&byte))
        {
            return Err(io::Error::other("unsupported laptop output name"));
        }
        let modules: Vec<Module> = self.list("modules")?;
        if self.journal.mirror.as_ref().is_some_and(|mirror| {
            mirror.sink == destination.name
                && modules.iter().any(|module| {
                    module.index == mirror.module
                        && module.name == "module-loopback"
                        && module.argument.split_whitespace().any(|argument| {
                            argument == format!("source={}.monitor", self.journal.sink)
                        })
                })
        }) {
            return Ok(());
        }
        self.remove_mirror()?;
        let module = String::from_utf8(self.run(&[
            "load-module",
            "module-loopback",
            &format!("source={}.monitor", self.journal.sink),
            &format!("sink={}", destination.name),
            "latency_msec=20",
            "source_dont_move=true",
            "sink_dont_move=true",
            "sink_input_properties=application.id=org.handover.audio-mirror",
        ])?)
        .map_err(io::Error::other)?
        .trim()
        .parse()
        .map_err(io::Error::other)?;
        self.journal.mirror = Some(SavedMirror {
            module,
            sink: destination.name.clone(),
        });
        self.save()
    }
    fn laptop_destination(&self, input: &Input, sinks: &[Sink]) -> Option<String> {
        let exists =
            |name: &str| name != self.journal.sink && sinks.iter().any(|sink| sink.name == name);
        if let Some(saved) = self
            .journal
            .inputs
            .get(&input.index)
            .filter(|saved| input.matches(saved))
            && exists(&saved.sink)
        {
            return Some(saved.sink.clone());
        }
        if exists(&self.journal.default_sink) {
            return Some(self.journal.default_sink.clone());
        }
        let current = String::from_utf8(self.run(&["get-default-sink"]).ok()?).ok()?;
        let current = current.trim();
        exists(current).then(|| current.to_owned())
    }
    fn route_apps(&mut self) -> io::Result<()> {
        let sinks: Vec<Sink> = self.list("sinks")?;
        let target = sinks
            .iter()
            .find(|sink| {
                sink.name == self.journal.sink && sink.owner_module == Some(self.journal.module)
            })
            .ok_or_else(|| io::Error::other("phone output disappeared"))?;
        let exclusive = self.settings.output == AudioOutputMode::Phone
            && self.settings.source == AudioSourceSelection::All;
        if exclusive {
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
                    self.run(&["set-sink-mute", &sink.name, "1"])?;
                }
            }
        } else {
            self.restore_mutes(&sinks)?;
        }
        self.reconcile_mirror(&sinks)?;
        let inputs: Vec<Input> = self.list("sink-inputs")?;
        if inputs.len() > 4096 {
            return Err(io::Error::other("too many audio streams"));
        }
        self.journal.inputs.retain(|index, saved| {
            inputs
                .iter()
                .any(|input| input.index == *index && input.matches(saved))
        });
        for input in inputs {
            if input.is_mirror(self.journal.mirror.as_ref()) {
                continue;
            }
            let selected = self.settings.output != AudioOutputMode::Laptop
                && match &self.settings.source {
                    AudioSourceSelection::All => true,
                    AudioSourceSelection::Applications { ids } => {
                        ids.contains(&input.application().id)
                    }
                };
            if !selected {
                if input.sink == target.index {
                    let destination = self
                        .laptop_destination(&input, &sinks)
                        .ok_or_else(|| io::Error::other("no laptop output available"))?;
                    self.run(&["move-sink-input", &input.index.to_string(), &destination])?;
                }
                self.journal.inputs.remove(&input.index);
                self.save()?;
                continue;
            }
            if input.sink == target.index {
                continue;
            }
            let Some(source) = sinks.iter().find(|sink| sink.index == input.sink) else {
                continue;
            };
            self.journal.inputs.insert(
                input.index,
                SavedInput {
                    sink: source.name.clone(),
                    client: input.client,
                    application: Some(input.application().id),
                },
            );
            self.save()?;
            if self
                .run(&[
                    "move-sink-input",
                    &input.index.to_string(),
                    &self.journal.sink,
                ])
                .is_err()
                && self
                    .list::<Input>("sink-inputs")?
                    .iter()
                    .any(|current| current.index == input.index && current.client == input.client)
            {
                return Err(io::Error::other("could not route app audio"));
            }
        }
        Ok(())
    }

    fn restore(&mut self) -> io::Result<()> {
        self.healthy.store(false, Ordering::Release);
        self.remove_mirror()?;
        let sinks: Vec<Sink> = self.list("sinks")?;
        let target = sinks
            .iter()
            .find(|s| s.name == self.journal.sink && s.owner_module == Some(self.journal.module));
        let mut failed = false;
        if let Some(target) = target {
            for input in self.list::<Input>("sink-inputs")? {
                if input.sink != target.index {
                    continue;
                }
                let destination = self.laptop_destination(&input, &sinks);
                if let Some(destination) = destination {
                    failed |= self
                        .run(&["move-sink-input", &input.index.to_string(), &destination])
                        .is_err();
                } else {
                    failed = true;
                }
            }
        }
        // Restore mute last so restoring an app cannot briefly play twice.
        for (name, saved) in &self.journal.muted {
            if sinks
                .iter()
                .any(|s| s.name == *name && s.index == saved.index)
            {
                failed |= self
                    .run(&["set-sink-mute", name, if saved.mute { "1" } else { "0" }])
                    .is_err();
            }
        }
        if !failed {
            if target.is_some() {
                self.run(&["unload-module", &self.journal.module.to_string()])?;
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
    server: Arc<dyn AudioServer>,
    pub(crate) settings: AudioRoutingSettings,
    settings_path: PathBuf,
    pub(crate) settings_available: bool,
    pub(crate) revision: u64,
    pub(crate) cached: Option<handover_core::AudioControlsSnapshot>,
    known_applications: BTreeMap<String, AudioApplication>,
}

impl OutputRouter {
    pub(crate) fn new(directory: &std::path::Path) -> Self {
        let mut router = Self {
            path: directory.join("audio-routing.json"),
            group: None,
            stopping: false,
            server: Arc::new(PulseServer),
            settings: AudioRoutingSettings::default(),
            settings_path: directory.join("audio-settings.json"),
            settings_available: true,
            revision: 0,
            cached: None,
            known_applications: BTreeMap::new(),
        };
        if router.settings_path.exists() {
            let loaded = std::fs::File::open(&router.settings_path).and_then(|file| {
                let mut bytes = Vec::new();
                file.take(16 * 1024 + 1).read_to_end(&mut bytes)?;
                if bytes.len() > 16 * 1024 {
                    return Err(io::Error::other("audio settings too large"));
                }
                serde_json::from_slice::<AudioRoutingSettings>(&bytes).map_err(io::Error::other)
            });
            match loaded {
                Ok(settings) if valid_settings(&settings) => router.settings = settings,
                _ => {
                    router.settings_available = false;
                    router.settings.output = AudioOutputMode::Laptop;
                    router.settings.source = AudioSourceSelection::Applications { ids: Vec::new() };
                }
            }
        }
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
            settings: self.settings.clone(),
            server: self.server.clone(),
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
            let default_sink = String::from_utf8(inner.server.run(&["get-default-sink"])?)
                .map_err(io::Error::other)?
                .trim()
                .to_owned();
            let module = String::from_utf8(inner.server.run(&[
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
                settings: inner.settings.clone(),
                server: inner.server.clone(),
                journal: Journal {
                    sink,
                    module,
                    default_sink,
                    muted: BTreeMap::new(),
                    inputs: BTreeMap::new(),
                    mirror: None,
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

    pub(crate) fn applications(&mut self) -> io::Result<Vec<AudioApplication>> {
        let inputs: Vec<Input> = server_list(self.server.as_ref(), "sink-inputs")?;
        if inputs.len() > 4096 {
            return Err(io::Error::other("too many audio streams"));
        }
        let mut observed = BTreeMap::new();
        for input in inputs {
            if input.is_mirror(None) {
                continue;
            }
            let app = input.application();
            observed.insert(app.id.clone(), app);
        }
        if let AudioSourceSelection::Applications { ids } = &self.settings.source {
            for id in ids {
                if !observed.contains_key(id) {
                    let mut app =
                        self.known_applications
                            .get(id)
                            .cloned()
                            .unwrap_or(AudioApplication {
                                id: id.clone(),
                                name: "Unavailable application".into(),
                                available: false,
                            });
                    app.available = false;
                    observed.insert(id.clone(), app);
                }
            }
        }
        if observed.len() > 128 {
            return Err(io::Error::other("too many audio applications"));
        }
        self.known_applications = observed.clone();
        let mut apps: Vec<_> = observed.into_values().collect();
        apps.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
        Ok(apps)
    }
    pub(crate) fn configure(&mut self, settings: AudioRoutingSettings) -> io::Result<()> {
        if !valid_settings(&settings) {
            return Err(io::Error::other("invalid audio settings"));
        }
        let old = self.settings.clone();
        if let Some(group) = &self.group {
            let mut route = group.route.lock().unwrap();
            route.settings = settings.clone();
            if let Err(error) = route.route_apps() {
                route.settings = old.clone();
                if route.route_apps().is_err() {
                    route.healthy.store(false, Ordering::Release);
                }
                return Err(error);
            }
        }
        let temporary = self.settings_path.with_extension("tmp");
        let persist = (|| {
            crate::identity::write_private(&temporary, &serde_json::to_vec(&settings)?)
                .map_err(io::Error::other)?;
            std::fs::rename(&temporary, &self.settings_path)
        })();
        if let Err(error) = persist {
            if let Some(group) = &self.group {
                let mut route = group.route.lock().unwrap();
                route.settings = old;
                if route.route_apps().is_err() {
                    route.healthy.store(false, Ordering::Release);
                }
            }
            let _ = std::fs::remove_file(temporary);
            return Err(error);
        }
        self.settings = settings;
        self.settings_available = true;
        self.revision += 1;
        Ok(())
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

fn valid_settings(settings: &AudioRoutingSettings) -> bool {
    let valid = |id: &str, prefix: &str| {
        id.strip_prefix(prefix)
            .is_some_and(|value| value.len() == 64 && value.bytes().all(|b| b.is_ascii_hexdigit()))
    };
    let source = match &settings.source {
        AudioSourceSelection::All => true,
        AudioSourceSelection::Applications { ids } => {
            ids.len() <= 64 && ids.iter().all(|id| valid(id, "app-"))
        }
    };
    let player = match &settings.playback {
        handover_core::AudioPlaybackTarget::Automatic => true,
        handover_core::AudioPlaybackTarget::Player { id } => valid(id, "player-"),
    };
    source && player
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct FakeAudioState {
        sinks: Vec<serde_json::Value>,
        inputs: Vec<serde_json::Value>,
        modules: Vec<serde_json::Value>,
        default_sink: String,
        commands: Vec<Vec<String>>,
        next_module: u32,
    }

    #[derive(Default)]
    struct FakeAudioServer(Mutex<FakeAudioState>);

    impl FakeAudioServer {
        fn seeded(inputs: Vec<serde_json::Value>, modules: Vec<serde_json::Value>) -> Self {
            Self(Mutex::new(FakeAudioState {
                sinks: vec![
                    serde_json::json!({"index": 1, "name": "laptop", "mute": false, "owner_module": null}),
                    serde_json::json!({"index": 2, "name": "handover_phone_test", "mute": false, "owner_module": 7}),
                ],
                inputs,
                modules,
                default_sink: "laptop".into(),
                commands: Vec::new(),
                next_module: 80,
            }))
        }

        fn input_sink(&self, index: u32) -> u32 {
            self.0
                .lock()
                .unwrap()
                .inputs
                .iter()
                .find(|input| input["index"] == index)
                .unwrap()["sink"]
                .as_u64()
                .unwrap() as u32
        }

        fn sink_mute(&self, name: &str) -> bool {
            self.0
                .lock()
                .unwrap()
                .sinks
                .iter()
                .find(|sink| sink["name"] == name)
                .unwrap()["mute"]
                .as_bool()
                .unwrap()
        }

        fn commands(&self) -> Vec<Vec<String>> {
            self.0.lock().unwrap().commands.clone()
        }
    }

    impl AudioServer for FakeAudioServer {
        fn run(&self, args: &[&str]) -> io::Result<Vec<u8>> {
            let mut state = self.0.lock().unwrap();
            state
                .commands
                .push(args.iter().map(|arg| (*arg).to_owned()).collect());
            match args {
                ["--format=json", "list", "sinks"] => Ok(serde_json::to_vec(&state.sinks)?),
                ["--format=json", "list", "sink-inputs"] => Ok(serde_json::to_vec(&state.inputs)?),
                ["--format=json", "list", "modules"] => Ok(serde_json::to_vec(&state.modules)?),
                ["get-default-sink"] => Ok(state.default_sink.as_bytes().to_vec()),
                ["set-sink-mute", name, value] => {
                    let sink = state
                        .sinks
                        .iter_mut()
                        .find(|sink| sink["name"] == *name)
                        .ok_or_else(|| io::Error::other("unknown fake sink"))?;
                    sink["mute"] = serde_json::Value::Bool(*value == "1");
                    Ok(Vec::new())
                }
                ["move-sink-input", index, destination] => {
                    let index: u32 = index.parse().map_err(io::Error::other)?;
                    let sink_index = state
                        .sinks
                        .iter()
                        .find(|sink| sink["name"] == *destination)
                        .and_then(|sink| sink["index"].as_u64())
                        .ok_or_else(|| io::Error::other("unknown fake sink"))?;
                    let input = state
                        .inputs
                        .iter_mut()
                        .find(|input| input["index"] == index)
                        .ok_or_else(|| io::Error::other("unknown fake input"))?;
                    input["sink"] = serde_json::json!(sink_index);
                    Ok(Vec::new())
                }
                ["load-module", "module-loopback", rest @ ..] => {
                    let index = state.next_module;
                    state.next_module += 1;
                    state.modules.push(serde_json::json!({
                        "index": index,
                        "name": "module-loopback",
                        "argument": rest.join(" "),
                    }));
                    Ok(index.to_string().into_bytes())
                }
                ["unload-module", index] => {
                    let index: u32 = index.parse().map_err(io::Error::other)?;
                    state.modules.retain(|module| module["index"] != index);
                    state
                        .sinks
                        .retain(|sink| sink["owner_module"] != serde_json::json!(index));
                    Ok(Vec::new())
                }
                _ => Err(io::Error::other(format!(
                    "unsupported fake pactl args: {args:?}"
                ))),
            }
        }
    }

    fn app_id(identity: &str) -> String {
        format!("app-{}", hex::encode(Sha256::digest(identity.as_bytes())))
    }

    fn app_input(index: u32, sink: u32, app: &str) -> serde_json::Value {
        serde_json::json!({
            "index": index, "sink": sink, "client": 10,
            "properties": {"application.id": app, "application.name": app}
        })
    }

    fn test_route(
        server: Arc<FakeAudioServer>,
        directory: &tempfile::TempDir,
        settings: AudioRoutingSettings,
    ) -> Route {
        Route {
            journal: Journal {
                sink: "handover_phone_test".into(),
                module: 7,
                default_sink: "laptop".into(),
                muted: BTreeMap::new(),
                inputs: BTreeMap::new(),
                mirror: None,
            },
            settings,
            server,
            path: directory.path().join("audio-routing.json"),
            healthy: Arc::new(AtomicBool::new(true)),
            restored: false,
        }
    }

    fn selective(ids: &[&str]) -> AudioSourceSelection {
        AudioSourceSelection::Applications {
            ids: ids.iter().map(|id| app_id(id)).collect(),
        }
    }

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
    fn selective_phone_routing_moves_only_selected_apps() {
        let server = Arc::new(FakeAudioServer::seeded(
            vec![
                app_input(11, 1, "selected.app"),
                app_input(12, 1, "local.app"),
            ],
            vec![],
        ));
        let directory = tempfile::tempdir().unwrap();
        let mut route = test_route(
            server.clone(),
            &directory,
            AudioRoutingSettings {
                source: selective(&["selected.app"]),
                output: AudioOutputMode::Phone,
                ..Default::default()
            },
        );

        route.route_apps().unwrap();

        assert_eq!(server.input_sink(11), 2);
        assert_eq!(server.input_sink(12), 1);
    }

    #[test]
    fn both_output_does_not_recapture_its_loopback_stream() {
        let mirror = serde_json::json!({
            "index": 90, "sink": 1, "client": 99, "owner_module": 41,
            "properties": {"application.id": "org.handover.audio-mirror", "application.name": "Audio mirror"}
        });
        let modules = vec![serde_json::json!({
            "index": 41, "name": "module-loopback",
            "argument": "source=handover_phone_test.monitor sink=laptop"
        })];
        let server = Arc::new(FakeAudioServer::seeded(
            vec![app_input(11, 1, "music.app"), mirror],
            modules,
        ));
        let directory = tempfile::tempdir().unwrap();
        let mut route = test_route(
            server.clone(),
            &directory,
            AudioRoutingSettings {
                source: AudioSourceSelection::All,
                output: AudioOutputMode::Both,
                ..Default::default()
            },
        );
        route.journal.mirror = Some(SavedMirror {
            module: 41,
            sink: "laptop".into(),
        });

        route.route_apps().unwrap();

        assert_eq!(server.input_sink(11), 2);
        assert_eq!(server.input_sink(90), 1);
        assert!(!route.journal.inputs.contains_key(&90));
    }

    #[test]
    fn laptop_mode_returns_selected_stream_to_its_original_sink() {
        let server = Arc::new(FakeAudioServer::seeded(
            vec![app_input(11, 2, "music.app")],
            vec![],
        ));
        let directory = tempfile::tempdir().unwrap();
        let mut route = test_route(
            server.clone(),
            &directory,
            AudioRoutingSettings {
                source: selective(&["music.app"]),
                output: AudioOutputMode::Laptop,
                ..Default::default()
            },
        );
        route.journal.inputs.insert(
            11,
            SavedInput {
                sink: "laptop".into(),
                client: Some(10),
                application: Some(app_id("music.app")),
            },
        );

        route.route_apps().unwrap();

        assert_eq!(server.input_sink(11), 1);
        assert!(!route.journal.inputs.contains_key(&11));
    }

    #[test]
    fn restore_uses_current_default_when_saved_and_original_outputs_disappear() {
        let server = Arc::new(FakeAudioServer::seeded(
            vec![app_input(11, 2, "music.app")],
            vec![],
        ));
        {
            let mut state = server.0.lock().unwrap();
            state.sinks.retain(|sink| sink["name"] != "laptop");
            state.sinks.push(serde_json::json!({
                "index": 3, "name": "new-default", "mute": false, "owner_module": null
            }));
            state.default_sink = "new-default".into();
        }
        let directory = tempfile::tempdir().unwrap();
        let mut route = test_route(server.clone(), &directory, AudioRoutingSettings::default());
        route.journal.inputs.insert(
            11,
            SavedInput {
                sink: "disconnected-output".into(),
                client: Some(10),
                application: Some(app_id("music.app")),
            },
        );

        route.restore().unwrap();

        assert_eq!(server.input_sink(11), 3);
        assert!(
            !server
                .0
                .lock()
                .unwrap()
                .sinks
                .iter()
                .any(|sink| sink["name"] == "handover_phone_test")
        );
    }

    #[test]
    fn restore_keeps_journal_and_temporary_sink_when_no_output_survives() {
        let server = Arc::new(FakeAudioServer::seeded(
            vec![app_input(11, 2, "music.app")],
            vec![],
        ));
        {
            let mut state = server.0.lock().unwrap();
            state
                .sinks
                .retain(|sink| sink["name"] == "handover_phone_test");
            state.default_sink = "disconnected-output".into();
        }
        let directory = tempfile::tempdir().unwrap();
        let mut route = test_route(server.clone(), &directory, AudioRoutingSettings::default());
        route.journal.inputs.insert(
            11,
            SavedInput {
                sink: "disconnected-output".into(),
                client: Some(10),
                application: Some(app_id("music.app")),
            },
        );
        route.save().unwrap();

        assert!(route.restore().is_err());

        assert!(
            server
                .0
                .lock()
                .unwrap()
                .sinks
                .iter()
                .any(|sink| sink["name"] == "handover_phone_test")
        );
        assert!(directory.path().join("audio-routing.json").exists());
        let journal: Journal = serde_json::from_slice(
            &std::fs::read(directory.path().join("audio-routing.json")).unwrap(),
        )
        .unwrap();
        assert!(journal.inputs.contains_key(&11));
        assert!(
            !server
                .commands()
                .iter()
                .any(|command| { command.first().is_some_and(|arg| arg == "unload-module") })
        );
        // Prevent Drop from retrying the intentionally failed recovery in this test.
        route.restored = true;
    }

    #[test]
    fn settings_load_marks_corruption_unavailable_and_preserves_valid_settings() {
        let invalid_directory = tempfile::tempdir().unwrap();
        std::fs::write(
            invalid_directory.path().join("audio-settings.json"),
            b"not valid settings",
        )
        .unwrap();

        let invalid = OutputRouter::new(invalid_directory.path());

        assert!(!invalid.settings_available);
        assert_eq!(invalid.settings.output, AudioOutputMode::Laptop);
        assert_eq!(
            invalid.settings.source,
            AudioSourceSelection::Applications { ids: Vec::new() }
        );

        let valid_directory = tempfile::tempdir().unwrap();
        let expected = AudioRoutingSettings {
            source: selective(&["music.app"]),
            output: AudioOutputMode::Both,
            ..Default::default()
        };
        std::fs::write(
            valid_directory.path().join("audio-settings.json"),
            serde_json::to_vec(&expected).unwrap(),
        )
        .unwrap();

        let valid = OutputRouter::new(valid_directory.path());

        assert!(valid.settings_available);
        assert_eq!(valid.settings, expected);
    }

    #[test]
    fn leaving_phone_all_restores_original_mutes_for_selective_and_both_modes() {
        for output in [AudioOutputMode::Phone, AudioOutputMode::Both] {
            let server = Arc::new(FakeAudioServer::seeded(vec![], vec![]));
            let directory = tempfile::tempdir().unwrap();
            let mut route = test_route(
                server.clone(),
                &directory,
                AudioRoutingSettings {
                    source: AudioSourceSelection::All,
                    output: AudioOutputMode::Phone,
                    ..Default::default()
                },
            );
            route.route_apps().unwrap();
            assert!(server.sink_mute("laptop"));
            assert!(!server.sink_mute("handover_phone_test"));

            route.settings = AudioRoutingSettings {
                source: selective(&["music.app"]),
                output,
                ..Default::default()
            };
            route.route_apps().unwrap();

            assert!(
                !server.sink_mute("laptop"),
                "original mute state must return for {output:?}"
            );
            assert!(route.journal.muted.is_empty());
        }
    }

    #[test]
    fn restore_does_not_unload_a_reused_module_index() {
        let modules = vec![
            serde_json::json!({"index": 7, "name": "module-combine-sink", "argument": "sink_name=other"}),
        ];
        let server = Arc::new(FakeAudioServer::seeded(vec![], modules));
        server
            .0
            .lock()
            .unwrap()
            .sinks
            .retain(|sink| sink["name"] != "handover_phone_test");
        let directory = tempfile::tempdir().unwrap();
        let mut route = test_route(server.clone(), &directory, AudioRoutingSettings::default());

        route.restore().unwrap();

        assert!(
            !server
                .commands()
                .iter()
                .any(|command| command.first().is_some_and(|arg| arg == "unload-module"))
        );
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
