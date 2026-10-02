import { eligibleTabUrl, makeRecord, matchRpcUrl, sanitizeContentType, shapeJsonText } from './privacy.mjs';
import { protobufShape } from './wire-shape.mjs';

const MAX_REQUESTS = 256;
const MAX_RECORDS = 512;
const MAX_BODY_BYTES = 1024 * 1024;
const MAX_BODY_CALLS = 8;
const BODY_TYPES = new Set(['application/x-protobuf', 'application/protobuf']);
const JSON_TYPES = new Set(['application/json', 'application/json+protobuf']);

let tabId = null;
let active = false;
let records = [];
let dropped = 0;
let durationTimer = null;
let bodyCalls = 0;
let generation = 0;
let commandQueue = Promise.resolve();
const requests = new Map();

function notify() {
  try { void chrome.runtime.sendMessage({ type: 'state-changed' }).catch(() => {}); } catch { /* popup may be closed */ }
}

function append(record, expectedGeneration = generation) {
  if (!record || !active || expectedGeneration !== generation) return;
  if (records.length >= MAX_RECORDS) { dropped += 1; return; }
  records.push(record);
  notify();
}

function byteCount(text) { return new TextEncoder().encode(text).length; }

function base64Length(encoded) {
  if (encoded.length > Math.ceil(MAX_BODY_BYTES * 4 / 3) + 4) return null;
  if (!/^(?:[A-Za-z0-9+/]{4})*(?:[A-Za-z0-9+/]{2}==|[A-Za-z0-9+/]{3}=)?$/.test(encoded)) return null;
  const padding = encoded.endsWith('==') ? 2 : encoded.endsWith('=') ? 1 : 0;
  return Math.max(0, (encoded.length / 4) * 3 - padding);
}

function decodeBase64(encoded, expectedBytes) {
  if (expectedBytes > MAX_BODY_BYTES || encoded.length > Math.ceil(MAX_BODY_BYTES * 4 / 3) + 4) return null;
  try {
    const binary = atob(encoded);
    const bytes = new Uint8Array(binary.length);
    for (let i = 0; i < binary.length; i += 1) bytes[i] = binary.charCodeAt(i);
    return bytes;
  } catch { return null; }
}

function requestBodyShape(request, contentType) {
  if (BODY_TYPES.has(contentType) && Array.isArray(request.postDataEntries)) {
    if (request.postDataEntries.length > 128) return { type: 'opaque', truncated: true };
    const encodedParts = [];
    let encodedTotal = 0;
    for (const entry of request.postDataEntries) {
      if (typeof entry?.bytes !== 'string') return { type: 'opaque' };
      encodedTotal += entry.bytes.length;
      if (encodedTotal > Math.ceil(MAX_BODY_BYTES * 4 / 3) + request.postDataEntries.length * 4) return { type: 'opaque', truncated: true };
      encodedParts.push(entry.bytes);
    }
    const lengths = encodedParts.map(base64Length);
    if (lengths.some(length => length === null)) return { type: 'opaque' };
    const size = lengths.reduce((total, length) => total + length, 0);
    if (size > MAX_BODY_BYTES) return { type: 'opaque', bytes: size, truncated: true };
    const bytes = new Uint8Array(size);
    let offset = 0;
    for (let i = 0; i < encodedParts.length; i += 1) {
      const part = decodeBase64(encodedParts[i], lengths[i]);
      if (!part) return { type: 'opaque', bytes: size };
      bytes.set(part, offset); offset += part.length;
    }
    return protobufShape(bytes);
  }
  if (typeof request.postData !== 'string') return undefined;
  if (BODY_TYPES.has(contentType)) return { type: 'opaque', representationBytes: byteCount(request.postData) };
  return shapeJsonText(request.postData);
}

function responseBodyShape(result, contentType) {
  const body = typeof result?.body === 'string' ? result.body : '';
  if (result?.base64Encoded) {
    const size = base64Length(body);
    if (size === null) return { type: 'opaque' };
    if (size > MAX_BODY_BYTES) return { type: 'opaque', bytes: size, truncated: true };
    const bytes = decodeBase64(body, size);
    if (!bytes) return { type: 'opaque', bytes: size };
    if (BODY_TYPES.has(contentType)) return protobufShape(bytes);
    if (JSON_TYPES.has(contentType)) {
      try { return shapeJsonText(new TextDecoder('utf-8', { fatal: true }).decode(bytes)); }
      catch { return { type: 'opaque', bytes: size }; }
    }
    return { type: 'opaque', bytes: size };
  }
  const size = byteCount(body);
  if (size > MAX_BODY_BYTES) return { type: 'opaque', bytes: size, truncated: true };
  if (JSON_TYPES.has(contentType)) return shapeJsonText(body);
  if (BODY_TYPES.has(contentType)) return { type: 'opaque', representationBytes: size };
  return { type: 'opaque', bytes: size };
}

async function stop(reason = 'Stopped.') {
  const oldTabId = tabId;
  active = false;
  tabId = null;
  generation += 1;
  if (durationTimer !== null) {
    clearTimeout(durationTimer);
    void chrome.alarms.clear('observer-duration');
  }
  durationTimer = null;
  requests.clear();
  if (oldTabId !== null) {
    try { await chrome.debugger.detach({ tabId: oldTabId }); } catch { /* already detached */ }
  }
  notify();
  return { message: reason };
}

async function start(duration) {
  if (!Number.isInteger(duration) || duration < 1 || duration > 300) return { error: 'Choose a duration from 1 to 300 seconds.' };
  const [tab] = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
  if (!tab || !Number.isInteger(tab.id) || !eligibleTabUrl(tab.url ?? '')) return { error: 'The active tab must be messages.google.com over HTTPS.' };
  if (active) await stop('Previous observation stopped.');
  generation += 1;
  records = [];
  dropped = 0;
  tabId = tab.id;
  try {
    await chrome.debugger.attach({ tabId }, '1.3');
    active = true;
    const [currentTab] = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
    if (!currentTab || currentTab.id !== tab.id || !eligibleTabUrl(currentTab.url ?? '')) throw new Error('tab_changed');
    await withTimeout(chrome.debugger.sendCommand({ tabId }, 'Network.enable', {
      maxTotalBufferSize: 2 * 1024 * 1024,
      maxResourceBufferSize: 1024 * 1024,
      maxPostDataSize: 1024 * 1024,
    }), 5000);
    durationTimer = setTimeout(() => { void enqueueCommand(() => stop('Duration ended.')); }, duration * 1000);
    chrome.alarms.create('observer-duration', { when: Date.now() + duration * 1000 });
    notify();
    return { message: `Observing for up to ${duration} seconds.` };
  } catch {
    await stop('Could not attach the observer.');
    return { error: 'Could not attach the observer.' };
  }
}

function withTimeout(promise, milliseconds) {
  let timeout;
  return Promise.race([
    promise,
    new Promise((_, reject) => { timeout = setTimeout(() => reject(new Error('command_timeout')), milliseconds); }),
  ]).finally(() => clearTimeout(timeout));
}

chrome.debugger.onEvent.addListener((source, method, params = {}) => {
  if (!active || source.tabId !== tabId || method !== 'Network.requestWillBeSent' &&
      method !== 'Network.responseReceived' && method !== 'Network.loadingFinished' && method !== 'Network.loadingFailed') return;
  if (method === 'Network.requestWillBeSent') {
    const request = params.request ?? {};
    const rpc = matchRpcUrl(request.url);
    if (!rpc || request.method === 'OPTIONS') return;
    const contentType = sanitizeContentType(request.headers);
    const key = params.requestId;
    if (requests.size >= MAX_REQUESTS) requests.delete(requests.keys().next().value);
    requests.set(key, { rpc, httpMethod: request.method, requestContentType: contentType });
    append(makeRecord({ phase: 'request', rpc, httpMethod: request.method, contentType, body: requestBodyShape(request, contentType) }));
    return;
  }
  if (method === 'Network.responseReceived') {
    const entry = requests.get(params.requestId);
    if (!entry) return;
    entry.status = params.response?.status;
    entry.responseContentType = sanitizeContentType(params.response?.headers);
    append(makeRecord({ phase: 'response', rpc: entry.rpc, httpMethod: entry.httpMethod, status: entry.status, contentType: entry.responseContentType }));
    return;
  }
  if (method === 'Network.loadingFailed') { requests.delete(params.requestId); return; }
  if (method === 'Network.loadingFinished') {
    const entry = requests.get(params.requestId);
    requests.delete(params.requestId);
    if (!entry) return;
    if (bodyCalls >= MAX_BODY_CALLS) {
      append(makeRecord({ phase: 'response', rpc: entry.rpc, body: { type: 'unavailable' } }));
      return;
    }
    bodyCalls += 1;
    const bodyGeneration = generation;
    let responseTimer;
    const timeout = new Promise((_, reject) => {
      responseTimer = setTimeout(() => reject(new Error('body_timeout')), 5000);
    });
    Promise.race([
      chrome.debugger.sendCommand(source, 'Network.getResponseBody', { requestId: params.requestId }),
      timeout,
    ])
      .then(result => {
        clearTimeout(responseTimer);
        if (!active || bodyGeneration !== generation) return;
        append(makeRecord({ phase: 'response', rpc: entry.rpc, body: responseBodyShape(result, entry.responseContentType) }), bodyGeneration);
      })
      .catch(() => {
        clearTimeout(responseTimer);
        if (active && bodyGeneration === generation) append(makeRecord({ phase: 'response', rpc: entry.rpc, body: { type: 'unavailable' } }), bodyGeneration);
      })
      .finally(() => { bodyCalls -= 1; });
  }
});

chrome.debugger.onDetach.addListener(source => {
  if (source.tabId !== tabId) return;
  active = false;
  generation += 1;
  tabId = null;
  if (durationTimer !== null) clearTimeout(durationTimer);
  if (durationTimer !== null) void chrome.alarms.clear('observer-duration');
  durationTimer = null;
  requests.clear();
  notify();
});

chrome.alarms.onAlarm.addListener(alarm => {
  if (alarm.name === 'observer-duration' && active) void enqueueCommand(() => stop('Duration ended.'));
});

chrome.tabs.onUpdated.addListener((updatedTabId, changeInfo) => {
  if (active && updatedTabId === tabId && typeof changeInfo.url === 'string' && !eligibleTabUrl(changeInfo.url)) {
    active = false;
    void enqueueCommand(() => stop('Stopped after the tab navigated away.'));
  }
});

chrome.tabs.onRemoved.addListener(removedTabId => {
  if (active && removedTabId === tabId) void enqueueCommand(() => stop('Stopped because the tab closed.'));
});

function enqueueCommand(operation) {
  const result = commandQueue.then(operation, operation);
  commandQueue = result.catch(() => {});
  return result;
}

chrome.runtime.onMessage.addListener((message, sender, sendResponse) => {
  if (sender.id !== chrome.runtime.id || sender.tab) return false;
  if (message?.type === 'snapshot') {
    sendResponse({ active, count: records.length, dropped, records: records.slice() });
    return false;
  }
  if (message?.type === 'start') { enqueueCommand(() => start(message.duration)).then(sendResponse, () => sendResponse({ error: 'Could not start the observer.' })); return true; }
  if (message?.type === 'stop') { enqueueCommand(() => stop()).then(sendResponse, () => sendResponse({ error: 'Could not stop the observer.' })); return true; }
  return false;
});
