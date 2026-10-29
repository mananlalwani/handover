.pragma library

function devices(items) {
    return items.filter(device => device.connected && device.paired
        && (device.capabilities || []).includes("file_transfer"));
}

function resource(value) {
    if (/^https?:\/\//i.test(value))
        return { method: "share.url", value: value };
    if (/^file:\/\//i.test(value))
        return { method: "share.file", value: value };
    if (value.startsWith("/"))
        return { method: "share.file", value: "file://" + value.split("/").map(encodeURIComponent).join("/") };
    return null;
}

function start(deviceId, method) {
    return { deviceId: deviceId, method: method, phase: "pending", transferId: "",
        text: "Submitting request…", early: [] };
}

function event(state, value, result) {
    if (!state || !value || value.device_id !== state.deviceId)
        return state;
    if (state.phase === "pending") {
        // Events can arrive before the command reply. Keep only a bounded set.
        const early = state.early.filter(item => item.value.transfer_id !== value.transfer_id);
        const previous = state.early.find(item => item.value.transfer_id === value.transfer_id);
        early.push(previous && previous.result ? previous : { value: value, result: result });
        return Object.assign({}, state, { early: early.slice(-32) });
    }
    if (state.phase !== "accepted" || !state.transferId || value.transfer_id !== state.transferId)
        return state;
    if (result) {
        if (value.status === "completed")
            return Object.assign({}, state, { phase: "done", text: "Transfer completed." });
        if (value.status === "failed")
            return Object.assign({}, state, { phase: "done", text: "Transfer failed: " + (value.reason || "unspecified") });
        return state;
    }
    return Object.assign({}, state, {
        text: "Sending: " + value.bytes_sent + " / " + value.total_bytes
            + " bytes written. Waiting for completion."
    });
}

function accepted(state, method, deviceId, transferId) {
    if (!state || state.phase !== "pending" || state.deviceId !== deviceId || state.method !== method)
        return state;
    let next = Object.assign({}, state, { phase: "accepted", transferId: transferId,
        early: [], text: "Request accepted. Completion is not confirmed." });
    for (const item of state.early)
        next = event(next, item.value, item.result);
    return next;
}

function failed(state, error) {
    if (!state || state.phase !== "pending")
        return state;
    return Object.assign({}, state, { phase: "done", early: [], text: "Request failed: " + error });
}

function disconnected(state) {
    if (!state || state.phase === "done")
        return state;
    return Object.assign({}, state, { phase: "done", early: [],
        text: "Daemon disconnected. Transfer outcome is unknown. Check the receiving device before sending again." });
}
