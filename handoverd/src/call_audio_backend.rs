//! Host-specific Bluetooth HFP call-audio routing.
//!
//! This module deliberately exposes normalized choices while keeping PulseAudio,
//! PipeWire and BlueZ identifiers in the daemon. It never changes global defaults.
use handover_core::{CallAudioChoice, CallAudioSelection};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, os::unix::process::CommandExt, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, BufReader},
    process::{Child, Command},
    time::timeout,
};

const OUTPUT_LIMIT: usize = 4 * 1024 * 1024;
const COMMAND_TIMEOUT: Duration = Duration::from_secs(3);
const PROFILE_TIMEOUT: Duration = Duration::from_secs(5);
static ROUTE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
const PROFILE_NAMES: &[&str] = &[
    "audio-gateway",
    "headset_audio_gateway",
    "headset-audio-gateway",
];

#[derive(Clone, Debug, Default)]
pub(crate) struct Inventory {
    pub endpoints: Vec<CallAudioChoice>,
    pub inputs: Vec<CallAudioChoice>,
    pub outputs: Vec<CallAudioChoice>,
    endpoints_private: BTreeMap<String, Endpoint>,
    inputs_private: BTreeMap<String, Node>,
    outputs_private: BTreeMap<String, Node>,
}

#[derive(Clone, Debug)]
struct Endpoint {
    card: String,
    index: u64,
    active_profile: String,
    gateway_profile: String,
}

#[derive(Clone, Debug)]
struct Node {
    name: String,
    card: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub(crate) struct ProfileRecovery {
    pub card: String,
    pub previous_profile: String,
    pub selected_profile: String,
}

/// Child handles own the loopback lifetimes; dropping the daemon drops both routes.
pub(crate) struct OwnedRoute {
    pub profile: ProfileRecovery,
    children: Vec<Child>,
    edges: Vec<(String, String)>,
}

#[derive(Deserialize)]
struct Card {
    index: u64,
    name: String,
    description: Option<String>,
    active_profile: Option<String>,
    profiles: BTreeMap<String, Value>,
    properties: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
struct AudioNode {
    name: String,
    description: Option<String>,
    card: Option<u64>,
    monitor_of_sink: Option<u64>,
}

pub(crate) async fn inventory() -> Result<Inventory, String> {
    let info: Value = serde_json::from_slice(&run("pactl", &["--format=json", "info"]).await?)
        .map_err(|_| "Audio server information is unavailable")?;
    if !info["server_name"]
        .as_str()
        .is_some_and(|name| name.to_ascii_lowercase().contains("pipewire"))
    {
        return Err(
            "Call audio requires PipeWire with its PulseAudio compatibility service".into(),
        );
    }
    let (cards, sources, sinks) = tokio::try_join!(list("cards"), list("sources"), list("sinks"))?;
    normalize(cards, sources, sinks).await
}

async fn list(kind: &'static str) -> Result<Vec<Value>, String> {
    let bytes = run("pactl", &["--format=json", "list", kind]).await?;
    serde_json::from_slice(&bytes).map_err(|_| format!("invalid pactl {kind} response"))
}

async fn normalize(
    cards: Vec<Value>,
    sources: Vec<Value>,
    sinks: Vec<Value>,
) -> Result<Inventory, String> {
    let cards: Vec<Card> = decode(cards, "card")?;
    let sources: Vec<AudioNode> = decode(sources, "source")?;
    let sinks: Vec<AudioNode> = decode(sinks, "sink")?;
    let mut result = Inventory::default();
    let mut bluetooth_cards = Vec::new();
    for card in cards {
        if !valid_node_name(&card.name) {
            continue;
        }
        let Some(path) = property_string(&card.properties, "api.bluez5.path") else {
            continue;
        };
        bluetooth_cards.push((card.index, card.name.clone()));
        let Some(address) = property_string(&card.properties, "api.bluez5.address")
            .or_else(|| property_string(&card.properties, "device.string"))
        else {
            continue;
        };
        if !bluez_device_connected(path, address).await {
            continue;
        }
        let Some(profile) = PROFILE_NAMES.iter().find(|name| {
            card.profiles.get(**name).is_some_and(|p| {
                p["n_sinks"].as_u64().unwrap_or(0) > 0 && p["n_sources"].as_u64().unwrap_or(0) > 0
            })
        }) else {
            continue;
        };
        let id = opaque_id("endpoint", &card.name);
        result.endpoints.push(CallAudioChoice {
            id: id.clone(),
            label: card
                .description
                .clone()
                .unwrap_or_else(|| "Bluetooth phone".into()),
        });
        result.endpoints_private.insert(
            id,
            Endpoint {
                card: card.name,
                index: card.index,
                active_profile: card.active_profile.unwrap_or_default(),
                gateway_profile: (*profile).to_string(),
            },
        );
    }
    let bt_indices: Vec<u64> = bluetooth_cards.iter().map(|(i, _)| *i).collect();
    for source in sources {
        if source.monitor_of_sink.is_some() || !local_node_name(&source.name) {
            continue;
        }
        if source.card.is_some_and(|i| bt_indices.contains(&i)) {
            continue;
        }
        let id = opaque_id("input", &source.name);
        result.inputs.push(CallAudioChoice {
            id: id.clone(),
            label: source
                .description
                .clone()
                .unwrap_or_else(|| "Microphone".into()),
        });
        result.inputs_private.insert(
            id,
            Node {
                name: source.name,
                card: source.card,
            },
        );
    }
    for sink in sinks {
        if !local_node_name(&sink.name) {
            continue;
        }
        if sink.card.is_some_and(|i| bt_indices.contains(&i)) {
            continue;
        }
        let id = opaque_id("output", &sink.name);
        result.outputs.push(CallAudioChoice {
            id: id.clone(),
            label: sink.description.clone().unwrap_or_else(|| "Speaker".into()),
        });
        result.outputs_private.insert(
            id,
            Node {
                name: sink.name,
                card: sink.card,
            },
        );
    }
    Ok(result)
}

fn valid_node_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 1024 && !name.chars().any(char::is_control)
}
fn local_node_name(name: &str) -> bool {
    valid_node_name(name)
        && !name.starts_with("handover_phone_")
        && !name.starts_with("handover-call-")
}

fn decode<T: for<'de> Deserialize<'de>>(values: Vec<Value>, what: &str) -> Result<Vec<T>, String> {
    values
        .into_iter()
        .map(|v| serde_json::from_value(v).map_err(|_| format!("invalid pactl {what} record")))
        .collect()
}

fn property_string<'a>(properties: &'a BTreeMap<String, Value>, key: &str) -> Option<&'a str> {
    properties
        .get(key)
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
}

async fn bluez_device_connected(path: &str, address: &str) -> bool {
    if !path.starts_with("/org/bluez/") || path.contains([' ', '\'', '"']) {
        return false;
    }
    let paired = run(
        "busctl",
        &[
            "--system",
            "get-property",
            "org.bluez",
            path,
            "org.bluez.Device1",
            "Paired",
        ],
    )
    .await;
    let connected = run(
        "busctl",
        &[
            "--system",
            "get-property",
            "org.bluez",
            path,
            "org.bluez.Device1",
            "Connected",
        ],
    )
    .await;
    let address_result = run(
        "busctl",
        &[
            "--system",
            "get-property",
            "org.bluez",
            path,
            "org.bluez.Device1",
            "Address",
        ],
    )
    .await;
    matches!(paired, Ok(ref b) if b == b"b true\n")
        && matches!(connected, Ok(ref b) if b == b"b true\n")
        && matches!(address_result, Ok(ref b) if String::from_utf8_lossy(b).trim() == format!("s \"{address}\""))
}

pub(crate) async fn start_route(
    inv: &Inventory,
    selection: &CallAudioSelection,
    active: &(dyn Fn() -> bool + Send + Sync),
    journal: &(dyn Fn(&ProfileRecovery) -> Result<(), String> + Send + Sync),
) -> Result<OwnedRoute, String> {
    let _ = &selection.device_id;
    let (endpoint, input, output) = selected_nodes(inv, selection)?;
    let profile = ProfileRecovery {
        card: endpoint.card.clone(),
        previous_profile: endpoint.active_profile.clone(),
        selected_profile: endpoint.gateway_profile.clone(),
    };
    if !(active)() {
        return Err("call ended before audio setup".into());
    }
    journal(&profile)?;

    // Subscribe before switching profiles so a fast node appearance is not missed.
    let mut subscription = subscribe()?;
    let mut route = OwnedRoute {
        profile,
        children: Vec::new(),
        edges: Vec::new(),
    };
    if route.profile.previous_profile != route.profile.selected_profile {
        if let Err(error) = run(
            "pactl",
            &[
                "set-card-profile",
                &route.profile.card,
                &route.profile.selected_profile,
            ],
        )
        .await
        {
            let _ = stop_subscription(&mut subscription).await;
            return Err(error);
        }
        if let Err(error) = wait_for_phone_nodes(endpoint.index, &mut subscription).await {
            let _ = stop_subscription(&mut subscription).await;
            let _ = restore_profile(&route.profile).await;
            return Err(error);
        }
    }
    let _ = stop_subscription(&mut subscription).await;
    let (phone_source, phone_sink) = match phone_nodes(endpoint.index).await {
        Ok(nodes) => nodes,
        Err(error) => {
            let _ = restore_profile(&route.profile).await;
            return Err(error);
        }
    };

    let marker = opaque_id(
        "route",
        &format!(
            "{}:{}:{}:{}",
            selection.endpoint_id,
            selection.input_id,
            selection.output_id,
            ROUTE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
        ),
    );
    let phone_to_local = loopback_args(&phone_source.name, &output.name, "phone-audio", &marker);
    if !(active)() {
        let _ = restore_profile(&route.profile).await;
        return Err("call ended before audio setup".into());
    }
    route.edges.extend(loopback_edges(&phone_to_local)?);
    match spawn_loopback(&phone_to_local) {
        Ok(child) => route.children.push(child),
        Err(error) => {
            let _ = restore_profile(&route.profile).await;
            return Err(error);
        }
    }

    let local_to_phone = loopback_args(&input.name, &phone_sink.name, "local-mic", &marker);
    if !(active)() {
        let _ = stop_route(&mut route).await;
        return Err("call ended before audio setup".into());
    }
    route.edges.extend(loopback_edges(&local_to_phone)?);
    match spawn_loopback(&local_to_phone) {
        Ok(child) => route.children.push(child),
        Err(error) => {
            let _ = stop_route(&mut route).await;
            return Err(error);
        }
    }
    let alive = processes_alive(&mut route).await;
    if !(active)() || !matches!(alive, Ok(true)) {
        let _ = stop_route(&mut route).await;
        return Err(alive
            .err()
            .unwrap_or_else(|| "call ended or audio process exited during setup".into()));
    }
    Ok(route)
}

fn selected_nodes<'a>(
    inv: &'a Inventory,
    selection: &CallAudioSelection,
) -> Result<(&'a Endpoint, &'a Node, &'a Node), String> {
    let endpoint = inv
        .endpoints_private
        .get(&selection.endpoint_id)
        .ok_or("unknown call audio endpoint")?;
    let input = inv
        .inputs_private
        .get(&selection.input_id)
        .ok_or("unknown call audio input")?;
    let output = inv
        .outputs_private
        .get(&selection.output_id)
        .ok_or("unknown call audio output")?;
    if input.card.is_some_and(|i| i == endpoint.index)
        || output.card.is_some_and(|i| i == endpoint.index)
    {
        return Err("local audio nodes cannot belong to the phone endpoint".into());
    }
    if !valid_node_name(&endpoint.card)
        || !local_node_name(&input.name)
        || !local_node_name(&output.name)
    {
        return Err("invalid audio node selection".into());
    }
    if input.name == output.name {
        return Err("input and output selections are inconsistent".into());
    }
    Ok((endpoint, input, output))
}

async fn phone_nodes(card: u64) -> Result<(Node, Node), String> {
    let sources: Vec<AudioNode> = decode(list("sources").await?, "source")?;
    let sinks: Vec<AudioNode> = decode(list("sinks").await?, "sink")?;
    let source = sources
        .into_iter()
        .find(|n| n.card == Some(card) && n.monitor_of_sink.is_none() && valid_node_name(&n.name))
        .ok_or("phone call source is not ready")?;
    let sink = sinks
        .into_iter()
        .find(|n| n.card == Some(card) && valid_node_name(&n.name))
        .ok_or("phone call sink is not ready")?;
    Ok((
        Node {
            name: source.name,
            card: source.card,
        },
        Node {
            name: sink.name,
            card: sink.card,
        },
    ))
}

async fn wait_for_phone_nodes(card: u64, subscription: &mut Child) -> Result<(), String> {
    let stdout = subscription
        .stdout
        .take()
        .ok_or("pactl subscribe has no output")?;
    let mut lines = BufReader::new(stdout).lines();
    let deadline = tokio::time::Instant::now() + PROFILE_TIMEOUT;
    loop {
        if phone_nodes(card).await.is_ok() {
            return Ok(());
        }
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        if remaining.is_zero() {
            return Err("Bluetooth call profile did not expose duplex audio nodes".into());
        }
        let line = timeout(remaining, lines.next_line())
            .await
            .map_err(|_| "Bluetooth call profile did not expose duplex audio nodes")?
            .map_err(|_| "pactl subscribe stream failed")?;
        let Some(line) = line else {
            return Err("pactl subscribe exited".into());
        };
        if line.contains(" card ") || line.contains(" source ") || line.contains(" sink ") {
            continue;
        }
    }
}

fn subscribe() -> Result<Child, String> {
    Command::new("pactl")
        .arg("subscribe")
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .map_err(|_| "pactl subscribe is unavailable".into())
}

async fn stop_subscription(subscription: &mut Child) -> Result<(), String> {
    let _ = subscription.kill().await;
    let _ = subscription.wait().await;
    Ok(())
}

fn loopback_args(capture: &str, playback: &str, direction: &str, marker: &str) -> Vec<String> {
    let id = format!("org.handover.call-audio.{direction}");
    let node_name = format!("handover-call-{marker}-{direction}");
    let capture_props = serde_json::json!({
        "node.dont-reconnect": true,
        "node.dont-fallback": true,
        "node.autoconnect": true,
        "target.object": capture,
        "application.id": id.clone(),
        "node.name": format!("{node_name}-capture"),
    });
    let playback_props = serde_json::json!({
        "node.dont-reconnect": true,
        "node.dont-fallback": true,
        "node.autoconnect": true,
        "target.object": playback,
        "application.id": id,
        "node.name": format!("{node_name}-playback"),
    });
    vec![
        "-n".into(),
        node_name,
        "-C".into(),
        capture.into(),
        "-P".into(),
        playback.into(),
        "-l".into(),
        "20".into(),
        "--capture-props".into(),
        capture_props.to_string(),
        "--playback-props".into(),
        playback_props.to_string(),
    ]
}

fn loopback_edges(args: &[String]) -> Result<Vec<(String, String)>, String> {
    let value = |name: &str| {
        args.iter()
            .position(|a| a == name)
            .and_then(|i| args.get(i + 1))
            .ok_or_else(|| "Audio routing arguments are incomplete".to_string())
    };
    let capture: Value = serde_json::from_str(value("--capture-props")?)
        .map_err(|_| "Invalid capture properties")?;
    let playback: Value = serde_json::from_str(value("--playback-props")?)
        .map_err(|_| "Invalid playback properties")?;
    Ok(vec![
        (
            value("-C")?.clone(),
            capture["node.name"]
                .as_str()
                .ok_or("Missing capture name")?
                .into(),
        ),
        (
            playback["node.name"]
                .as_str()
                .ok_or("Missing playback name")?
                .into(),
            value("-P")?.clone(),
        ),
    ])
}
fn graph_routes_running(graph: &[Value], edges: &[(String, String)]) -> bool {
    let nodes: BTreeMap<_, _> = graph
        .iter()
        .filter(|v| v["type"] == "PipeWire:Interface:Node")
        .filter_map(|v| {
            Some((
                v["info"]["props"]["node.name"].as_str()?,
                (v["id"].as_u64()?, v["info"]["state"].as_str()?),
            ))
        })
        .collect();
    !edges.is_empty()
        && edges.iter().all(|(source, sink)| {
            let (Some((source, source_state)), Some((sink, sink_state))) =
                (nodes.get(source.as_str()), nodes.get(sink.as_str()))
            else {
                return false;
            };
            *source_state == "running"
                && *sink_state == "running"
                && graph.iter().any(|v| {
                    v["type"] == "PipeWire:Interface:Link"
                        && v["info"]["state"] == "active"
                        && v["info"]["output-node-id"].as_u64() == Some(*source)
                        && v["info"]["input-node-id"].as_u64() == Some(*sink)
                })
        })
}

fn spawn_loopback(args: &[String]) -> Result<Child, String> {
    let parent = std::process::id() as libc::pid_t;
    let mut command = Command::new("pw-loopback");
    command
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true);
    unsafe {
        command.as_std_mut().pre_exec(move || {
            if libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGKILL) != 0 {
                return Err(std::io::Error::last_os_error());
            }
            if libc::getppid() != parent {
                // Keep the post-fork path allocation-free.
                return Err(std::io::Error::from_raw_os_error(libc::ESRCH));
            }
            Ok(())
        });
    }
    command
        .spawn()
        .map_err(|_| "pw-loopback is unavailable".into())
}

pub(crate) async fn processes_alive(route: &mut OwnedRoute) -> Result<bool, String> {
    for child in &mut route.children {
        if child
            .try_wait()
            .map_err(|_| "failed checking pw-loopback process")?
            .is_some()
        {
            return Ok(false);
        }
    }
    Ok(route.children.len() == 2)
}

pub(crate) async fn stop_route(route: &mut OwnedRoute) -> Result<(), String> {
    for child in &mut route.children {
        if child
            .try_wait()
            .map_err(|_| "failed checking pw-loopback process")?
            .is_none()
        {
            let _ = child.kill().await;
        }
        let _ = child.wait().await;
    }
    route.children.clear();
    restore_profile(&route.profile).await
}

pub(crate) async fn route_present(route: &mut OwnedRoute) -> Result<bool, String> {
    if !processes_alive(route).await? {
        return Ok(false);
    }
    let graph: Vec<Value> = serde_json::from_slice(&run("pw-dump", &[] as &[&str]).await?)
        .map_err(|_| "Invalid PipeWire graph")?;
    Ok(graph_routes_running(&graph, &route.edges))
}

pub(crate) async fn restore_profile(profile: &ProfileRecovery) -> Result<(), String> {
    if profile.previous_profile.is_empty() || profile.previous_profile == profile.selected_profile {
        return Ok(());
    }
    let cards: Vec<Value> = list("cards").await?;
    let Some(card) = cards
        .iter()
        .find(|c| c["name"] == profile.card && c["active_profile"] == profile.selected_profile)
    else {
        return Ok(());
    };
    let Some(index) = card["index"].as_u64() else {
        return Ok(());
    };
    let sources: Vec<Value> = list("sources").await?;
    let sinks: Vec<Value> = list("sinks").await?;
    let source_ids: Vec<u64> = sources
        .iter()
        .filter(|n| n["card"].as_u64() == Some(index) && n["monitor_of_sink"].is_null())
        .filter_map(|n| n["index"].as_u64())
        .collect();
    let sink_ids: Vec<u64> = sinks
        .iter()
        .filter(|n| n["card"].as_u64() == Some(index))
        .filter_map(|n| n["index"].as_u64())
        .collect();
    let sink_inputs: Vec<Value> = list("sink-inputs").await?;
    let source_outputs: Vec<Value> = list("source-outputs").await?;
    let external_sink_use = sink_inputs.iter().any(|stream| {
        stream["sink"]
            .as_u64()
            .is_some_and(|id| sink_ids.contains(&id))
    });
    let external_source_use = source_outputs.iter().any(|stream| {
        stream["source"]
            .as_u64()
            .is_some_and(|id| source_ids.contains(&id))
    });
    if external_sink_use || external_source_use {
        return Ok(());
    }
    run(
        "pactl",
        &["set-card-profile", &profile.card, &profile.previous_profile],
    )
    .await?;
    Ok(())
}

async fn run(program: &str, args: &[impl AsRef<std::ffi::OsStr>]) -> Result<Vec<u8>, String> {
    timeout(COMMAND_TIMEOUT, async {
        let mut child = Command::new(program)
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .map_err(|_| format!("{program} is unavailable"))?;
        let mut output = Vec::new();
        child
            .stdout
            .take()
            .ok_or("failed to capture command output")?
            .take((OUTPUT_LIMIT + 1) as u64)
            .read_to_end(&mut output)
            .await
            .map_err(|_| format!("failed reading {program} output"))?;
        if output.len() > OUTPUT_LIMIT {
            return Err(format!("{program} output exceeded limit"));
        }
        let status = child
            .wait()
            .await
            .map_err(|_| format!("failed waiting for {program}"))?;
        if !status.success() {
            return Err(format!("{program} command failed"));
        }
        Ok(output)
    })
    .await
    .map_err(|_| format!("{program} command timed out"))?
}

fn opaque_id(kind: &str, value: &str) -> String {
    format!("{kind}:{}", hex::encode(Sha256::digest(value.as_bytes())))
}

#[cfg(test)]
mod tests {
    use super::*;
    use handover_core::DeviceId;

    #[test]
    fn opaque_ids_are_stable_without_revealing_backend_names() {
        let id = opaque_id("input", "mic with space");
        assert_eq!(id, opaque_id("input", "mic with space"));
        assert!(!id.contains("mic"));
        assert!(!id.contains(' '));
    }

    #[test]
    fn loopback_arguments_keep_both_directions_explicit_and_json_safe() {
        let args = loopback_args(
            "private source",
            "speaker \"one\"",
            "local-mic",
            "route:123",
        );
        assert_eq!(args[0], "-n");
        assert_eq!(args[2], "-C");
        assert_eq!(args[3], "private source");
        assert_eq!(args[4], "-P");
        assert_eq!(args[5], "speaker \"one\"");
        let capture: Value = serde_json::from_str(&args[9]).unwrap();
        let playback: Value = serde_json::from_str(&args[11]).unwrap();
        assert_eq!(capture["target.object"], "private source");
        assert_eq!(playback["target.object"], "speaker \"one\"");
        assert_eq!(capture["node.dont-reconnect"], true);
        assert_eq!(playback["node.dont-fallback"], true);
    }

    #[test]
    fn selection_requires_observed_local_nodes_and_rejects_phone_nodes() {
        let mut inventory = Inventory::default();
        inventory.endpoints_private.insert(
            "ep".into(),
            Endpoint {
                card: "phone-card".into(),
                index: 7,
                active_profile: "a2dp".into(),
                gateway_profile: "audio-gateway".into(),
            },
        );
        inventory.inputs_private.insert(
            "mic".into(),
            Node {
                name: "local-mic".into(),
                card: Some(2),
            },
        );
        inventory.outputs_private.insert(
            "speaker".into(),
            Node {
                name: "local-speaker".into(),
                card: Some(3),
            },
        );
        let selection = CallAudioSelection {
            expected_revision: 1,
            device_id: DeviceId::new("device"),
            endpoint_id: "ep".into(),
            input_id: "mic".into(),
            output_id: "speaker".into(),
        };
        assert!(selected_nodes(&inventory, &selection).is_ok());
        inventory.inputs_private.get_mut("mic").unwrap().card = Some(7);
        assert!(selected_nodes(&inventory, &selection).is_err());
        inventory.inputs_private.get_mut("mic").unwrap().card = Some(2);
        let mut unknown = selection;
        unknown.output_id = "not-in-inventory".into();
        assert!(selected_nodes(&inventory, &unknown).is_err());
    }

    #[test]
    fn running_nodes_without_the_selected_links_are_not_routing_confirmation() {
        let mut graph = vec![
            serde_json::json!({"id":1,"type":"PipeWire:Interface:Node","info":{"state":"running","props":{"node.name":"phone-source"}}}),
            serde_json::json!({"id":2,"type":"PipeWire:Interface:Node","info":{"state":"running","props":{"node.name":"own-capture"}}}),
            serde_json::json!({"id":3,"type":"PipeWire:Interface:Node","info":{"state":"running","props":{"node.name":"own-playback"}}}),
            serde_json::json!({"id":4,"type":"PipeWire:Interface:Node","info":{"state":"running","props":{"node.name":"selected-speakers"}}}),
            serde_json::json!({"id":5,"type":"PipeWire:Interface:Node","info":{"state":"running","props":{"node.name":"other-speakers"}}}),
            serde_json::json!({"type":"PipeWire:Interface:Link","info":{"state":"active","output-node-id":1,"input-node-id":2}}),
            serde_json::json!({"type":"PipeWire:Interface:Link","info":{"state":"active","output-node-id":3,"input-node-id":5}}),
        ];
        let edges = vec![
            ("phone-source".into(), "own-capture".into()),
            ("own-playback".into(), "selected-speakers".into()),
        ];
        assert!(!graph_routes_running(&graph, &edges));
        graph.push(serde_json::json!({"type":"PipeWire:Interface:Link","info":{"state":"active","output-node-id":3,"input-node-id":4}}));
        assert!(graph_routes_running(&graph, &edges));
        graph[3]["info"]["state"] = Value::String("idle".into());
        assert!(!graph_routes_running(&graph, &edges));
    }

    #[test]
    #[ignore = "invoked in a subprocess by routing_processes_are_owned_and_die_with_the_parent"]
    fn call_audio_parent_helper() {
        use std::io::Write;
        let mode = std::env::var("HANDOVER_CALL_AUDIO_TEST_HELPER").unwrap();
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap();
        runtime.block_on(async {
            let child = spawn_loopback(&[]).unwrap();
            let pid = child.id().unwrap();
            println!("OWNED_ROUTE_PID={pid}");
            std::io::stdout().flush().unwrap();
            if mode == "death" {
                std::mem::forget(child);
            } else {
                let mut route = OwnedRoute {
                    profile: ProfileRecovery {
                        card: "fixture".into(),
                        previous_profile: String::new(),
                        selected_profile: String::new(),
                    },
                    children: vec![child],
                    edges: Vec::new(),
                };
                stop_route(&mut route).await.unwrap();
                assert!(route.children.is_empty());
            }
        });
        std::process::exit(0);
    }

    #[test]
    fn routing_processes_are_owned_and_die_with_the_parent() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().unwrap();
        let executable = directory.path().join("pw-loopback");
        std::fs::write(&executable, "#!/bin/sh\nexec /bin/sleep 60\n").unwrap();
        std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut unrelated = std::process::Command::new("/bin/sleep")
            .arg("60")
            .spawn()
            .unwrap();
        for mode in ["stop", "death"] {
            let path = std::env::join_paths(std::iter::once(directory.path().to_path_buf()).chain(
                std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default()),
            ))
            .unwrap();
            let result = std::process::Command::new(std::env::current_exe().unwrap())
                .args([
                    "--exact",
                    "call_audio_backend::tests::call_audio_parent_helper",
                    "--ignored",
                    "--nocapture",
                ])
                .env("PATH", path)
                .env("HANDOVER_CALL_AUDIO_TEST_HELPER", mode)
                .output()
                .unwrap();
            assert!(
                result.status.success(),
                "{}",
                String::from_utf8_lossy(&result.stderr)
            );
            let output = String::from_utf8(result.stdout).unwrap();
            let pid = output
                .lines()
                .find_map(|line| line.strip_prefix("OWNED_ROUTE_PID="))
                .unwrap()
                .parse::<u32>()
                .unwrap();
            let ended = || {
                std::fs::read_to_string(format!("/proc/{pid}/stat"))
                    .ok()
                    .is_none_or(|s| {
                        s.rsplit_once(')')
                            .is_some_and(|(_, tail)| tail.trim_start().starts_with('Z'))
                    })
            };
            let deadline = std::time::Instant::now() + Duration::from_secs(3);
            while !ended() && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert!(ended(), "owned audio process survived {mode}");
            assert!(
                unrelated.try_wait().unwrap().is_none(),
                "cleanup touched an unrelated process"
            );
        }
        let _ = unrelated.kill();
        let _ = unrelated.wait();
    }
}
