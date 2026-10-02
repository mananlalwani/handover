#!/usr/bin/env node
// Read-only, bounded observer for Google Messages RPC traffic over local CDP.
import { constants } from 'node:fs';
import { lstat, open } from 'node:fs/promises';
import { pathToFileURL } from 'node:url';

const RPC_RE = /^\/\$rpc\/google\.internal\.communications\.instantmessaging\.v1\.([A-Za-z0-9_]+)\/([A-Za-z0-9_]+)$/;
const MAX_NODES = 512;
const MAX_DEPTH = 6;
const MAX_ARRAY = 128;
const MAX_BODY = 1024 * 1024;
const MAX_TRACKED = 256;
const MAX_RECORDS = 512;
const MAX_DEVTOOLS_FILE = 4096;
const MAX_PENDING_CDP = 64;
const MAX_RESPONSE_BODIES = 8;
const MAX_HANDLERS = 512;

export function validateDevToolsAddress(contents) {
  if (typeof contents !== 'string' || Buffer.byteLength(contents, 'utf8') > MAX_DEVTOOLS_FILE) throw new Error('invalid_devtools_file');
  const lines = contents.trim().split(/\r?\n/);
  if (lines.length !== 2 || !/^\d+$/.test(lines[0])) throw new Error('invalid_devtools_file');
  const port = Number(lines[0]);
  const path = lines[1];
  if (!Number.isInteger(port) || port < 1 || port > 65535 ||
      !/^\/devtools\/browser\/[A-Za-z0-9._-]+$/.test(path)) {
    throw new Error('invalid_devtools_address');
  }
  return `ws://127.0.0.1:${port}${path}`;
}

export function matchRpcUrl(input) {
  try {
    const url = new URL(input);
    if (url.protocol !== 'https:' || url.username || url.password || url.port) return null;
    const host = url.hostname.toLowerCase();
    if (host !== 'messages.google.com' &&
        !host.endsWith('.googleapis.com') &&
        !host.endsWith('.clients6.google.com')) return null;
    const match = RPC_RE.exec(url.pathname);
    return match ? { service: match[1], method: match[2] } : null;
  } catch { return null; }
}

function typeName(value) {
  if (value === null) return 'null';
  if (Array.isArray(value)) return 'array';
  if (typeof value === 'object') return 'object';
  return typeof value;
}

export function bodyShape(input) {
  let value = input;
  if (typeof input === 'string') {
    if (Buffer.byteLength(input, 'utf8') > MAX_BODY) return { type: 'opaque', bytes: Buffer.byteLength(input), truncated: true };
    try { value = JSON.parse(input); } catch {
      return { type: 'opaque', bytes: Buffer.byteLength(input, 'utf8') };
    }
  }
  let nodes = 0;
  let truncated = false;
  function shape(item, depth) {
    nodes += 1;
    if (nodes > MAX_NODES) { truncated = true; return { type: 'truncated' }; }
    const type = typeName(item);
    if (type === 'string') return { type, bytes: Buffer.byteLength(item, 'utf8') };
    if (type !== 'object' && type !== 'array') return { type };
    if (depth >= MAX_DEPTH) { truncated = true; return { type, truncated: true }; }
    if (Array.isArray(item)) {
      const count = Math.min(item.length, MAX_ARRAY);
      if (item.length > count) truncated = true;
      const items = [];
      for (let i = 0; i < count && nodes < MAX_NODES; i += 1) items.push(shape(item[i], depth + 1));
      if (items.length < item.length) truncated = true;
      return { type, length: item.length, items };
    }
    const count = Object.keys(item).length;
    const values = [];
    for (const key of Object.keys(item).slice(0, MAX_NODES)) {
      if (nodes >= MAX_NODES) { truncated = true; break; }
      values.push(shape(item[key], depth + 1));
    }
    if (values.length < count) truncated = true;
    return { type, propertyCount: count, values };
  }
  const result = shape(value, 0);
  if (truncated) result.truncated = true;
  return result;
}

export function makeRpcRecord({ url, rpc: suppliedRpc, method, status, contentType, shape, phase }) {
  const rpc = suppliedRpc ?? matchRpcUrl(url);
  if (!rpc) return null;
  const record = { type: phase, service: rpc.service, method: rpc.method };
  if (method && ['POST', 'GET', 'OPTIONS'].includes(method)) record.httpMethod = method;
  if (Number.isInteger(status)) record.status = status;
  if (typeof contentType === 'string' && contentType.length <= 200) record.contentType = contentType;
  if (shape) record.body = shape;
  return record;
}

function headerValue(headers, name) {
  for (const [key, value] of Object.entries(headers ?? {})) {
    if (key.toLowerCase() === name) return String(value).slice(0, 200);
  }
  return undefined;
}

function parseCli(argv) {
  const opts = { duration: 60 };
  for (let i = 0; i < argv.length; i += 1) {
    if (argv[i] === '--devtools-file' && argv[i + 1]) opts.devtoolsFile = argv[++i];
    else if (argv[i] === '--output' && argv[i + 1]) opts.output = argv[++i];
    else if (argv[i] === '--duration' && argv[i + 1]) opts.duration = Number(argv[++i]);
    else throw new Error('invalid_arguments');
  }
  if (!opts.devtoolsFile || !opts.output || !Number.isInteger(opts.duration) || opts.duration < 1 || opts.duration > 600) {
    throw new Error('invalid_arguments');
  }
  return opts;
}

async function run(argv = process.argv.slice(2)) {
  const opts = parseCli(argv);
  const fileStat = await lstat(opts.devtoolsFile);
  if (!fileStat.isFile() || fileStat.isSymbolicLink()) throw new Error('invalid_devtools_file');
  if (typeof process.getuid === 'function' && fileStat.uid !== process.getuid()) throw new Error('invalid_devtools_file');
  if ((fileStat.mode & 0o077) !== 0) throw new Error('invalid_devtools_file');
  if (fileStat.size > MAX_DEVTOOLS_FILE) throw new Error('invalid_devtools_file');
  const devtoolsFile = await open(opts.devtoolsFile, constants.O_RDONLY | (constants.O_NOFOLLOW ?? 0));
  let devtoolsContents;
  try {
    const openedStat = await devtoolsFile.stat();
    if (!openedStat.isFile() || openedStat.size > MAX_DEVTOOLS_FILE) throw new Error('invalid_devtools_file');
    const bytes = Buffer.alloc(MAX_DEVTOOLS_FILE + 1);
    const { bytesRead } = await devtoolsFile.read(bytes, 0, bytes.length, 0);
    if (bytesRead > MAX_DEVTOOLS_FILE) throw new Error('invalid_devtools_file');
    devtoolsContents = bytes.subarray(0, bytesRead).toString('utf8');
  } finally { await devtoolsFile.close(); }
  const address = validateDevToolsAddress(devtoolsContents);
  const output = await open(opts.output, 'wx', 0o600);
  await output.chmod(0o600);
  const ws = new WebSocket(address);
  const pending = new Map();
  const targets = new Set();
  const requests = new Map();
  let nextId = 0;
  let records = 0;
  let dropped = 0;
  let responseBodies = 0;
  let acceptingEvents = true;
  let outputClosing = false;
  let writeQueue = Promise.resolve();
  const handlers = new Set();

  function send(method, params = {}, sessionId) {
    if (ws.readyState !== WebSocket.OPEN) return Promise.reject(new Error('cdp_closed'));
    if (pending.size >= MAX_PENDING_CDP) return Promise.reject(new Error('cdp_busy'));
    const id = ++nextId;
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => { pending.delete(id); reject(new Error('cdp_timeout')); }, 5000);
      pending.set(id, { resolve, reject, timer });
      ws.send(JSON.stringify({ id, method, params, ...(sessionId ? { sessionId } : {}) }));
    });
  }

  async function write(record) {
    if (!record || outputClosing) return;
    if (records >= MAX_RECORDS) { dropped += 1; return; }
    records += 1;
    writeQueue = writeQueue.then(() => output.write(`${JSON.stringify(record)}\n`));
    await writeQueue;
  }

  async function attach(target) {
    if (!target || target.type !== 'page' || targets.has(target.targetId)) return;
    // Target eligibility is page URL based; only HTTPS messages.google.com pages may attach.
    let parsed;
    try { parsed = new URL(target.url); } catch { return; }
    if (parsed.hostname.toLowerCase() !== 'messages.google.com' || parsed.protocol !== 'https:' ||
        parsed.username || parsed.password || parsed.port) return;
    targets.add(target.targetId);
    try {
      const { sessionId } = await send('Target.attachToTarget', { targetId: target.targetId, flatten: true });
      await send('Network.enable', {}, sessionId);
    } catch { targets.delete(target.targetId); }
  }

  async function responseBody(sessionId, requestId, entry) {
    if (responseBodies >= MAX_RESPONSE_BODIES) {
      await write({ type: 'response', service: entry.rpc.service, method: entry.rpc.method, body: { type: 'unavailable' } });
      return;
    }
    responseBodies += 1;
    try {
      const result = await send('Network.getResponseBody', { requestId }, sessionId);
      let bytes = result.body;
      if (result.base64Encoded) {
        if (bytes.length > Math.ceil(MAX_BODY * 4 / 3) + 8) {
          await write({ type: 'response', service: entry.rpc.service, method: entry.rpc.method, body: { type: 'opaque', truncated: true } }); return;
        }
        const decoded = Buffer.from(bytes, 'base64');
        if (decoded.length > MAX_BODY) { await write({ type: 'response', service: entry.rpc.service, method: entry.rpc.method, body: { type: 'opaque', bytes: decoded.length, truncated: true } }); return; }
        bytes = decoded.toString('utf8');
      } else if (Buffer.byteLength(bytes, 'utf8') > MAX_BODY) {
        await write({ type: 'response', service: entry.rpc.service, method: entry.rpc.method, body: { type: 'opaque', bytes: Buffer.byteLength(bytes), truncated: true } }); return;
      }
      // Strip only the documented anti-XSSI marker when it begins the payload.
      if (bytes.startsWith(")]}'")) bytes = bytes.replace(/^\)\]\}'(?:\r?\n)?/, '');
      await write({ type: 'response', service: entry.rpc.service, method: entry.rpc.method, body: bodyShape(bytes) });
    } catch {
      await write({ type: 'response', service: entry.rpc.service, method: entry.rpc.method, body: { type: 'unavailable' } });
    } finally { responseBodies -= 1; }
  }

  async function handleMessage(event) {
    let message;
    try { message = JSON.parse(event.data); } catch { return; }
    if (message.id) {
      const call = pending.get(message.id);
      if (!call) return;
      clearTimeout(call.timer); pending.delete(message.id);
      if (message.error) call.reject(new Error('cdp_error'));
      else call.resolve(message.result ?? {});
      return;
    }
    if (!acceptingEvents) return;
    const p = message.params ?? {};
    if (message.method === 'Target.targetCreated' || message.method === 'Target.targetInfoChanged') { await attach(p.targetInfo); return; }
    if (message.method === 'Target.targetDestroyed') { targets.delete(p.targetId); return; }
    if (!message.sessionId) return;
    if (message.method === 'Network.requestWillBeSent') {
      const request = p.request ?? {};
      const rpc = matchRpcUrl(request.url);
      if (!rpc) return;
      const requestKey = `${message.sessionId}:${p.requestId}`;
      if (requests.size >= MAX_TRACKED) requests.delete(requests.keys().next().value);
      requests.set(requestKey, { rpc, method: request.method });
      const record = makeRpcRecord({ rpc, method: request.method, contentType: headerValue(request.headers, 'content-type'), shape: typeof request.postData === 'string' ? bodyShape(request.postData) : undefined, phase: 'request' });
      await write(record);
    } else if (message.method === 'Network.responseReceived') {
      const requestKey = `${message.sessionId}:${p.requestId}`;
      const entry = requests.get(requestKey);
      if (!entry) return;
      const response = p.response ?? {};
      entry.status = response.status;
      entry.responseContentType = headerValue(response.headers, 'content-type');
      entry.sessionId = message.sessionId;
      const record = makeRpcRecord({ rpc: entry.rpc, method: entry.method, status: response.status, contentType: entry.responseContentType, phase: 'response' });
      await write(record);
    } else if (message.method === 'Network.loadingFinished') {
      const requestKey = `${message.sessionId}:${p.requestId}`;
      const entry = requests.get(requestKey);
      if (!entry || !entry.sessionId) return;
      requests.delete(requestKey);
      await responseBody(message.sessionId, p.requestId, entry);
    } else if (message.method === 'Network.loadingFailed') requests.delete(`${message.sessionId}:${p.requestId}`);
  }

  const onMessage = event => {
    if (handlers.size >= MAX_HANDLERS) return;
    const task = Promise.resolve().then(() => handleMessage(event)).catch(() => {}).finally(() => handlers.delete(task));
    handlers.add(task);
  };
  ws.addEventListener('message', onMessage);

  try {
    await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error('cdp_connect_timeout')), 5000);
      ws.addEventListener('open', () => { clearTimeout(timer); resolve(); }, { once: true });
      ws.addEventListener('error', () => { clearTimeout(timer); reject(new Error('cdp_connect_failed')); }, { once: true });
    });
    await send('Target.setDiscoverTargets', { discover: true });
    const listed = await send('Target.getTargets');
    for (const target of listed.targetInfos ?? []) await attach(target);
    await new Promise(resolve => { setTimeout(resolve, opts.duration * 1000); });
  } finally {
    acceptingEvents = false;
    let drainTimer;
    try {
      await Promise.race([
        Promise.allSettled([...handlers]),
        new Promise(resolve => { drainTimer = setTimeout(resolve, 5000); }),
      ]);
    } finally { clearTimeout(drainTimer); }
    ws.removeEventListener('message', onMessage);
    outputClosing = true;
    await writeQueue.catch(() => {});
    for (const call of pending.values()) { clearTimeout(call.timer); call.reject(new Error('cdp_closed')); }
    pending.clear();
    if (ws.readyState !== WebSocket.CLOSED) ws.close();
    await output.close();
  }
  process.stdout.write(`${JSON.stringify({ records, dropped, output: opts.output })}\n`);
}

if (process.argv[1] && import.meta.url === pathToFileURL(process.argv[1]).href) {
  run().catch(() => { process.stderr.write('observe_rpc: failed\n'); process.exitCode = 1; });
}
