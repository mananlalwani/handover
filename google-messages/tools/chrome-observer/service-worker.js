import { browserLookupRequest, eligibleTabUrl, makeRecord, matchGaiaProbeUrl, matchRpcUrl, sanitizeContentType, shapeJsonText } from './privacy.mjs';
import { protobufShape } from './wire-shape.mjs';

const MAX_REQUESTS = 256;
const MAX_RECORDS = 512;
const MAX_BODY_BYTES = 1024 * 1024;
const MAX_BODY_CALLS = 8;
const MAX_AUTH_CANDIDATES = 8;
const MAX_AUTH_HEADERS = 8;
const AUTH_PROBE_HOST = 'com.handover.google_messages.auth_probe';
const MAX_NATIVE_WAIT = 20_000;
const BODY_TYPES = new Set(['application/x-protobuf', 'application/protobuf']);
const JSON_TYPES = new Set(['application/json', 'application/json+protobuf']);
const GA_EMAIL_EXPRESSION = '(()=>{try{const app=globalThis.default_mw;const setting=app?.uE;const read=app?.u;if(!setting||typeof read!=="function")return null;return read(setting)}catch{return null}})()';

let tabId = null;
let active = false;
let records = [];
let dropped = 0;
let durationTimer = null;
let bodyCalls = 0;
let generation = 0;
let commandQueue = Promise.resolve();
let captureMode = null;
let captureServiceCookies = false;
let captureBrowserRequest = false;
let inspectLocalError = false;
let pairingAccountEmail = null;
let localDescription = null;
let localDescriptionTimer = null;

function clearLocalDescription() {
  localDescription = null;
  if (localDescriptionTimer !== null) clearTimeout(localDescriptionTimer);
  localDescriptionTimer = null;
}
let nativeProbe = { state: 'idle' };
let authForwarded = false;
let probeId = 0;
const requests = new Map();
const authCandidates = new Map();
const authHeaderMaps = new Map();

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

async function stop(reason = 'Stopped.', { preserveNativeProbe = false } = {}) {
  if (!preserveNativeProbe) probeId += 1;
  const oldTabId = tabId;
  const oldMode = captureMode;
  active = false;
  tabId = null;
  captureMode = null;
  captureServiceCookies = false;
  captureBrowserRequest = false;
  inspectLocalError = false;
  pairingAccountEmail = null;
  clearLocalDescription();
  generation += 1;
  if (durationTimer !== null) {
    clearTimeout(durationTimer);
    void chrome.alarms.clear('observer-duration');
  }
  durationTimer = null;
  requests.clear();
  authCandidates.clear();
  authHeaderMaps.clear();
  if (!preserveNativeProbe && (oldMode === 'auth' || oldMode === 'register' || oldMode === 'pairing_check' || oldMode === 'native_login') && nativeProbe.state === 'waiting') {
    const error = reason === 'Duration ended.' ? (oldMode === 'register' ? 'sign_in_not_observed' : 'timeout')
      : reason === 'Stopped after the tab navigated away.' ? 'tab_changed'
        : reason === 'Stopped because the tab closed.' ? 'tab_closed' : 'stopped';
    nativeProbe = { state: 'failed', result: { error, ...(oldMode === 'register' ? { registration: true } : {}) } };
  }
  if (oldTabId !== null) {
    try { await withTimeout(chrome.debugger.detach({ tabId: oldTabId }), 5000); } catch { /* already detached or timed out */ }
  }
  notify();
  return { message: reason };
}

async function start(duration, mode = 'observe', withServiceCookies = false, withBrowserRequest = false, inspect = false) {
  clearLocalDescription();
  probeId += 1;
  const maximum = mode === 'auth' || mode === 'register' || mode === 'pairing_check' || mode === 'native_login' ? 120 : 300;
  if (!Number.isInteger(duration) || duration < 1 || duration > maximum) return { error: `Choose a duration from 1 to ${maximum} seconds.` };
  const [tab] = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
  if (!tab || !Number.isInteger(tab.id) || !eligibleTabUrl(tab.url ?? '')) return { error: 'The active tab must be messages.google.com over HTTPS.' };
  if (active) await stop('Previous observation stopped.');
  generation += 1;
  if (mode === 'auth' || mode === 'register' || mode === 'pairing_check' || mode === 'native_login') {
    authForwarded = false;
    nativeProbe = { state: 'waiting', ...(mode === 'pairing_check' ? { operation: 'pairing_check' } : mode === 'native_login' ? { operation: 'native_login' } : {}) };
  }
  records = [];
  dropped = 0;
  tabId = tab.id;
  try {
    await chrome.debugger.attach({ tabId }, '1.3');
    active = true;
    captureMode = mode;
    captureServiceCookies = (mode === 'auth' || mode === 'register' || mode === 'pairing_check' || mode === 'native_login') && withServiceCookies === true;
    captureBrowserRequest = mode === 'auth' && withBrowserRequest === true;
    inspectLocalError = mode === 'auth' && inspect === true;
    const [currentTab] = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
    if (!currentTab || currentTab.id !== tab.id || !eligibleTabUrl(currentTab.url ?? '')) throw new Error('tab_changed');
    if (mode === 'pairing_check' || mode === 'native_login') {
      pairingAccountEmail = await readGoogleAccountEmail(tab.id);
      if (!pairingAccountEmail) throw new Error('account_unavailable');
    }
    await withTimeout(chrome.debugger.sendCommand({ tabId }, 'Network.enable', {
      maxTotalBufferSize: 2 * 1024 * 1024,
      maxResourceBufferSize: 1024 * 1024,
      maxPostDataSize: mode === 'auth' || mode === 'register' || mode === 'pairing_check' || mode === 'native_login' ? (captureBrowserRequest ? 2048 : 0) : 1024 * 1024,
    }), 5000);
    durationTimer = setTimeout(() => { void enqueueCommand(() => stop('Duration ended.')); }, duration * 1000);
    chrome.alarms.create('observer-duration', { when: Date.now() + duration * 1000 });
    notify();
    return { message: mode === 'register' ? 'Registration is waiting for SignInGaia.' : mode === 'pairing_check' ? 'Read-only source lookup is waiting for SignInGaia.' : mode === 'native_login' ? 'Native login is waiting for SignInGaia.' : mode === 'auth' ? 'Authentication probe is waiting for SignInGaia.' : `Observing for up to ${duration} seconds.` };
  } catch (error) {
    await stop('Could not attach the observer.');
    if (error?.message === 'account_unavailable') return { error: 'Could not read GA_EMAIL from the current Messages page. No native request was sent.' };
    return { error: 'Could not attach the observer.' };
  }
}

export function withTimeout(promise, milliseconds, errorCode = 'command_timeout') {
  let timeout;
  return Promise.race([
    promise,
    new Promise((_, reject) => { timeout = setTimeout(() => reject(new Error(errorCode)), milliseconds); }),
  ]).finally(() => clearTimeout(timeout));
}

function validAccountEmail(value) {
  return typeof value === 'string' && value.length <= 254 &&
    /^[A-Za-z0-9.!#$%&'*+/=?^_`{|}~-]{1,64}@[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?(?:\.[A-Za-z0-9](?:[A-Za-z0-9-]{0,61}[A-Za-z0-9])?)+$/.test(value);
}

// Read only Google's GA_EMAIL setting from the active Messages page. The fixed
// expression returns one string; it never enumerates page state or runs input.
export async function readGoogleAccountEmail(id) {
  const result = await withTimeout(chrome.debugger.sendCommand(
    { tabId: id },
    'Runtime.evaluate',
    { expression: GA_EMAIL_EXPRESSION, returnByValue: true, awaitPromise: false, silent: true },
  ), 5000);
  const email = result?.result?.value;
  if (result?.result) result.result.value = null;
  return validAccountEmail(email) ? email : null;
}

function selectedAuthHeaders(headers, includeServiceCookie = false) {
  const selected = {};
  if (!headers || typeof headers !== 'object') return selected;
  let count = 0;
  const read = (rawName, rawValue) => {
    count += 1;
    const name = String(rawName).toLowerCase();
    if (name === 'cookie' && !includeServiceCookie) return;
    if (!['authorization', 'x-goog-api-key', 'x-goog-authuser', 'origin', 'cookie'].includes(name)) return;
    if (typeof rawValue !== 'string') { selected[name] = null; return; }
    if (name === 'authorization' || name === 'x-goog-api-key' || name === 'cookie') {
      const max = name === 'cookie' ? 16384 : name === 'authorization' ? 8192 : 4096;
      selected[name] = rawValue.length > 0 && rawValue.length <= max && /^[\x20-\x7e]+$/.test(rawValue) ? rawValue : null;
    } else if (name === 'x-goog-authuser') {
      selected[name] = /^\d{1,2}$/.test(rawValue) && Number(rawValue) <= 99 ? rawValue : null;
    } else {
      try {
        if (rawValue.length > 256 || !/^[\x21-\x7e]+$/.test(rawValue)) { selected[name] = null; return; }
        const origin = new URL(rawValue);
        selected[name] = origin.protocol === 'https:' && origin.hostname.toLowerCase() === 'messages.google.com' &&
          !origin.username && !origin.password && !origin.port && origin.pathname === '/' && !origin.search && !origin.hash
          ? 'https://messages.google.com' : null;
      } catch { selected[name] = null; }
    }
  };
  if (Array.isArray(headers)) {
    for (let i = 0; i < headers.length && count < 2048; i += 1) read(headers[i]?.name, headers[i]?.value);
  } else {
    for (const name in headers) {
      if (count >= 2048) break;
      if (Object.hasOwn(headers, name)) read(name, headers[name]);
    }
  }
  return selected;
}

function clearAuthRequest(requestId) {
  authCandidates.delete(requestId);
  authHeaderMaps.delete(requestId);
}

async function processAuthCandidate(requestId, candidate) {
  if (!active || !['auth', 'register', 'pairing_check', 'native_login'].includes(captureMode) || authForwarded || !candidate || !candidate.ready) return;
  const headers = { ...candidate.requestHeaders, ...(candidate.extraHeaders ?? {}) };
  let authorization = headers.authorization;
  let apiKey = headers['x-goog-api-key'];
  let authUser = headers['x-goog-authuser'];
  let origin = headers.origin;
  if (typeof authorization !== 'string' || typeof apiKey !== 'string' ||
      Object.hasOwn(headers, 'origin') && origin !== 'https://messages.google.com' ||
      Object.hasOwn(headers, 'x-goog-authuser') && typeof authUser !== 'string') {
    clearAuthRequest(requestId);
    return;
  }
  if (candidate.withServiceCookies && typeof headers.cookie !== 'string') {
    authForwarded = true;
    nativeProbe = { state: 'failed', result: { error: 'cookie_unavailable' } };
    for (const name of Object.keys(headers)) headers[name] = null;
    candidate.requestHeaders = {};
    candidate.extraHeaders = {};
    await enqueueCommand(() => stop('Service cookie unavailable.', { preserveNativeProbe: true }));
    return;
  }
  const payload = {
    type: candidate.registration ? 'gaia_register' : candidate.nativeLogin ? 'gaia_login' : candidate.pairingCheck ? 'gaia_pairing' : candidate.inspect ? 'gaia_lookup_inspect' : candidate.browserRequest ? 'gaia_lookup_browser_request' : candidate.withServiceCookies ? 'gaia_lookup_with_cookies' : 'gaia_lookup',
    endpoint: candidate.endpoint,
    origin: origin ?? 'https://messages.google.com',
    authorization,
    api_key: apiKey,
    ...(authUser === undefined ? {} : { auth_user: authUser }),
    ...(candidate.withServiceCookies ? { service_cookie: headers.cookie } : {}),
    ...(candidate.pairingCheck || candidate.nativeLogin ? { account_email: pairingAccountEmail } : {}),
    ...(candidate.browserRequest ? { browser_request: candidate.browserRequest } : {}),
  };
  authForwarded = true;
  const operation = candidate.pairingCheck ? 'pairing_check' : candidate.nativeLogin ? 'native_login' : null;
  nativeProbe = { state: 'running', ...(operation ? { operation } : {}) };
  const currentProbe = probeId;
  const inspect = candidate.inspect;
  authCandidates.clear();
  authHeaderMaps.clear();
  candidate.requestHeaders = {};
  candidate.extraHeaders = {};
  candidate.browserRequest = null;
  for (const name of Object.keys(headers)) headers[name] = null;
  authorization = null;
  apiKey = null;
  authUser = null;
  origin = null;
  pairingAccountEmail = null;
  await enqueueCommand(() => stop('Authentication request captured.', { preserveNativeProbe: true }));
  if (currentProbe !== probeId) { clearNativePayload(payload); return; }

  try {
    let nativeCall;
    try { nativeCall = chrome.runtime.sendNativeMessage(AUTH_PROBE_HOST, payload); }
    finally { clearNativePayload(payload); }
    const outcome = await nativeReplyWithTimeout(Promise.resolve(nativeCall));
    if (currentProbe !== probeId) return;
    nativeProbe = outcome.reply ? sanitizeNativeReply(outcome.reply, candidate.registration === true) : { state: 'failed', result: { error: outcome.error, ...(candidate.registration ? { registration: true } : {}) } };
    if (operation) nativeProbe.operation = operation;
    if (candidate.registration && nativeProbe.state === 'failed') nativeProbe.result.registration = true;
    if (inspect && nativeProbe.state === 'failed' && nativeProbe.result?.error === 'http_error' &&
        typeof outcome.reply?.local_description === 'string' && byteCount(outcome.reply.local_description) <= 2048) {
      localDescription = outcome.reply.local_description;
      localDescriptionTimer = setTimeout(clearLocalDescription, 60_000);
    }
  } catch {
    if (currentProbe === probeId) nativeProbe = { state: 'failed', result: { error: 'native_error' } };
  } finally { notify(); }
}

function clearNativePayload(payload) {
  payload.authorization = '';
  payload.api_key = '';
  if (Object.hasOwn(payload, 'account_email')) payload.account_email = '';
  if (Object.hasOwn(payload, 'service_cookie')) delete payload.service_cookie;
  if (Object.hasOwn(payload, 'auth_user')) delete payload.auth_user;
  if (Object.hasOwn(payload, 'browser_request')) delete payload.browser_request;
}

export async function nativeReplyWithTimeout(nativeCall, milliseconds = MAX_NATIVE_WAIT) {
  try { return { reply: await withTimeout(Promise.resolve(nativeCall), milliseconds, 'native_timeout') }; }
  catch (error) { return { error: error?.message === 'native_timeout' ? 'timeout' : 'native_error' }; }
}

export function sanitizeNativeReply(value, registration = false) {
  const rpcStatuses = new Set(['CANCELLED', 'UNKNOWN', 'INVALID_ARGUMENT', 'DEADLINE_EXCEEDED', 'NOT_FOUND', 'ALREADY_EXISTS', 'PERMISSION_DENIED', 'UNAUTHENTICATED', 'RESOURCE_EXHAUSTED', 'FAILED_PRECONDITION', 'ABORTED', 'OUT_OF_RANGE', 'UNIMPLEMENTED', 'INTERNAL', 'UNAVAILABLE', 'DATA_LOSS']);
  const rpcReasons = new Set(['API_KEY_INVALID', 'API_KEY_SERVICE_BLOCKED', 'API_KEY_HTTP_REFERRER_BLOCKED', 'API_KEY_IP_ADDRESS_BLOCKED', 'API_KEY_ANDROID_APP_BLOCKED', 'API_KEY_IOS_APP_BLOCKED', 'CONSUMER_INVALID', 'SERVICE_DISABLED']);
  const errors = new Set(['invalid_origin', 'invalid_frame', 'invalid_bootstrap', 'invalid_endpoint', 'invalid_credentials', 'registration_failed', 'no_pending_registration', 'ambiguous_registration', 'daemon_unavailable', 'sign_in_not_observed', 'network', 'http_error', 'unexpected_response', 'response_too_large', 'timeout', 'rpc_error', 'native_error']);
  if (!value || typeof value !== 'object' || typeof value.ok !== 'boolean') {
    return { state: 'failed', result: { error: 'invalid_response' } };
  }
  if (value.ok) {
    if (!Number.isInteger(value.sources) || value.sources < 0 || value.sources > 128 ||
        value.error !== undefined || value.http_status !== undefined || value.rpc_status !== undefined || value.rpc_reason !== undefined) return { state: 'failed', result: { error: 'invalid_response' } };
    return { state: 'complete', result: registration
      ? { ok: true, registered: true }
      : { ok: true, sources: value.sources } };
  }
  if (typeof value.error !== 'string' || !errors.has(value.error)) return { state: 'failed', result: { error: 'invalid_response' } };
  const result = { ok: false, error: value.error };
  if (value.http_status !== undefined) {
    if (value.error !== 'http_error' || !Number.isInteger(value.http_status) || value.http_status < 100 || value.http_status > 599) {
      return { state: 'failed', result: { error: 'invalid_response' } };
    }
    result.http_status = value.http_status;
  }
  if (value.rpc_status !== undefined) {
    if (value.error !== 'http_error' || !rpcStatuses.has(value.rpc_status)) return { state: 'failed', result: { error: 'invalid_response' } };
    result.rpc_status = value.rpc_status;
  }
  if (value.rpc_reason !== undefined) {
    if (value.error !== 'http_error' || !rpcStatuses.has(value.rpc_status) || !rpcReasons.has(value.rpc_reason)) return { state: 'failed', result: { error: 'invalid_response' } };
    result.rpc_reason = value.rpc_reason;
  }
  return { state: 'failed', result };
}

function authRequest(requestId, request) {
  const endpoint = matchGaiaProbeUrl(request.url);
  if (!endpoint || request.method !== 'POST' || authForwarded) {
    authHeaderMaps.delete(requestId);
    return;
  }
  if (authCandidates.size >= MAX_AUTH_CANDIDATES && !authCandidates.has(requestId)) {
    authHeaderMaps.delete(requestId);
    return;
  }
  const browserRequest = captureBrowserRequest ? browserLookupRequest(request.postData) : null;
  if (captureBrowserRequest && !browserRequest) {
    clearAuthRequest(requestId);
    return;
  }
  const candidate = {
    endpoint, withServiceCookies: captureServiceCookies,
    registration: captureMode === 'register',
    pairingCheck: captureMode === 'pairing_check',
    nativeLogin: captureMode === 'native_login',
    browserRequest,
    inspect: inspectLocalError,
    requestHeaders: selectedAuthHeaders(request.headers, captureServiceCookies),
  };
  const prior = authHeaderMaps.get(requestId);
  if (prior) {
    candidate.extraHeaders = prior;
    candidate.ready = true;
    authHeaderMaps.delete(requestId);
  }
  authCandidates.set(requestId, candidate);
  if (candidate.ready) void processAuthCandidate(requestId, candidate);
}

function authExtraInfo(requestId, headers) {
  const candidate = authCandidates.get(requestId);
  // ExtraInfo can precede the URL event. Never retain cookies until the exact
  // service URL has been matched, even when the user enabled the comparison.
  const selected = selectedAuthHeaders(headers, candidate?.withServiceCookies === true);
  if (candidate) {
    candidate.extraHeaders = selected;
    candidate.ready = true;
    void processAuthCandidate(requestId, candidate);
  } else {
    if (authHeaderMaps.size >= MAX_AUTH_HEADERS && !authHeaderMaps.has(requestId)) authHeaderMaps.delete(authHeaderMaps.keys().next().value);
    authHeaderMaps.set(requestId, selected);
  }
}

chrome.debugger.onEvent.addListener((source, method, params = {}) => {
  if (!active || source.tabId !== tabId || method !== 'Network.requestWillBeSent' &&
      method !== 'Network.requestWillBeSentExtraInfo' && method !== 'Network.responseReceived' &&
      method !== 'Network.loadingFinished' && method !== 'Network.loadingFailed') return;
  if (captureMode === 'auth' || captureMode === 'register' || captureMode === 'pairing_check' || captureMode === 'native_login') {
    if (method === 'Network.requestWillBeSent') authRequest(params.requestId, params.request ?? {});
    else if (method === 'Network.requestWillBeSentExtraInfo') authExtraInfo(params.requestId, params.headers);
    else if (method === 'Network.responseReceived' && params.hasExtraInfo === false) {
      const candidate = authCandidates.get(params.requestId);
      if (candidate) { candidate.ready = true; void processAuthCandidate(params.requestId, candidate); }
    }
    else if (method === 'Network.loadingFailed') clearAuthRequest(params.requestId);
    return;
  }
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
  if ((captureMode === 'auth' || captureMode === 'register' || captureMode === 'pairing_check' || captureMode === 'native_login') && nativeProbe.state === 'waiting') nativeProbe = { state: 'failed', result: { error: 'detached' } };
  active = false;
  generation += 1;
  tabId = null;
  if (durationTimer !== null) clearTimeout(durationTimer);
  if (durationTimer !== null) void chrome.alarms.clear('observer-duration');
  durationTimer = null;
  requests.clear();
  authCandidates.clear();
  authHeaderMaps.clear();
  captureServiceCookies = false;
  captureBrowserRequest = false;
  inspectLocalError = false;
  pairingAccountEmail = null;
  clearLocalDescription();
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
    sendResponse({ active, mode: captureMode, count: records.length, dropped, records: records.slice(), nativeProbe: structuredClone(nativeProbe) });
    return false;
  }
  if (message?.type === 'take-local-description') {
    sendResponse({ description: localDescription });
    clearLocalDescription();
    return false;
  }
  if (message?.type === 'start') { enqueueCommand(() => start(message.duration)).then(sendResponse, () => sendResponse({ error: 'Could not start the observer.' })); return true; }
  if (message?.type === 'auth-probe') { enqueueCommand(() => start(120, 'auth')).then(sendResponse, () => sendResponse({ error: 'Could not start authentication probe.' })); return true; }
  if (message?.type === 'register-device') { enqueueCommand(() => start(120, 'register', true)).then(sendResponse, () => sendResponse({ error: 'Could not start registration.' })); return true; }
  if (message?.type === 'pairing-check') { enqueueCommand(() => start(120, 'pairing_check', true)).then(sendResponse, () => sendResponse({ error: 'Could not start pairing readiness check.' })); return true; }
  if (message?.type === 'native-login') { enqueueCommand(() => start(120, 'native_login', true)).then(sendResponse, () => sendResponse({ error: 'Could not start native login.' })); return true; }
  if (message?.type === 'auth-probe-with-cookies') { enqueueCommand(() => start(120, 'auth', true)).then(sendResponse, () => sendResponse({ error: 'Could not start authentication probe.' })); return true; }
  if (message?.type === 'auth-probe-browser-request') { enqueueCommand(() => start(120, 'auth', true, true)).then(sendResponse, () => sendResponse({ error: 'Could not start authentication probe.' })); return true; }
  if (message?.type === 'auth-probe-inspect') { enqueueCommand(() => start(120, 'auth', true, true, true)).then(sendResponse, () => sendResponse({ error: 'Could not start authentication probe.' })); return true; }
  if (message?.type === 'stop') { enqueueCommand(() => stop()).then(sendResponse, () => sendResponse({ error: 'Could not stop the observer.' })); return true; }
  return false;
});
