const RPC_RE = /^\/\$rpc\/google\.internal\.communications\.instantmessaging\.v1\.([A-Za-z0-9_]+)\/([A-Za-z0-9_]+)$/;
const MAX_BODY_BYTES = 1024 * 1024;
const MAX_NODES = 512;
const MAX_DEPTH = 6;
const MAX_ARRAY = 128;
const GAIA_PROBE_HOSTS = new Set([
  'instantmessaging-pa.googleapis.com',
  'instantmessaging-pa.clients6.google.com',
  'instantmessaging-pa-jms-us.clients6.google.com',
]);

export function matchRpcUrl(input) {
  try {
    if (typeof input !== 'string' || input.length > 8192) return null;
    const url = new URL(input);
    if (url.protocol !== 'https:' || url.username || url.password || url.port) return null;
    const host = url.hostname.toLowerCase();
    if (host !== 'messages.google.com' && !host.endsWith('.googleapis.com') && !host.endsWith('.clients6.google.com')) return null;
    const match = RPC_RE.exec(url.pathname);
    return match ? { service: match[1], method: match[2] } : null;
  } catch { return null; }
}

export function eligibleTabUrl(input) {
  try {
    if (typeof input !== 'string' || input.length > 8192) return false;
    const url = new URL(input);
    return url.protocol === 'https:' && url.hostname.toLowerCase() === 'messages.google.com' &&
      !url.username && !url.password && !url.port;
  } catch { return false; }
}

export function matchGaiaProbeUrl(input) {
  try {
    if (typeof input !== 'string' || input.length > 8192) return null;
    const url = new URL(input);
    if (url.protocol !== 'https:' || url.username || url.password || url.port ||
        !GAIA_PROBE_HOSTS.has(url.hostname.toLowerCase()) ||
        url.pathname !== '/$rpc/google.internal.communications.instantmessaging.v1.Registration/SignInGaia') return null;
    return url.origin;
  } catch { return null; }
}

// This optional comparison may forward only a token-free mode-1 lookup.
// Every occupied position has a known role; opaque identifiers stay transient.
export function browserLookupRequest(text) {
  if (typeof text !== 'string' || text.length > 2048) return null;
  try {
    const body = JSON.parse(text);
    const uuid = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;
    if (!Array.isArray(body) || body.length !== 4 || body[2] !== 1 || body[3] !== 'GDitto') return null;
    const header = body[0];
    if (!Array.isArray(header) || header.length !== 7 || typeof header[0] !== 'string' || !uuid.test(header[0]) || header[2] !== 'GDitto' || [1, 3, 4, 5].some(i => header[i] !== null)) return null;
    const info = header[6];
    if (!Array.isArray(info) || info.length !== 9 || [0, 1, 5, 7].some(i => info[i] !== null) || info[6] !== 4 || info[8] !== 6 || [2, 3, 4].some(i => !Number.isInteger(info[i]) || info[i] < 0 || info[i] > 0xffffffff)) return null;
    const device = body[1];
    if (!Array.isArray(device) || device.length !== 1 || !Array.isArray(device[0]) || device[0].length !== 2 || device[0][0] !== 3 || typeof device[0][1] !== 'string') return null;
    const suffix = device[0][1].startsWith('messages-web-') ? device[0][1].slice(13) : '';
    if (!/^[0-9a-f]{32}$/i.test(suffix) && !uuid.test(suffix)) return null;
    return body;
  } catch { return null; }
}

function typeName(value) {
  if (value === null) return 'null';
  if (Array.isArray(value)) return 'array';
  if (typeof value === 'object') return 'object';
  return typeof value;
}

export function shapeJsonValue(value) {
  let nodes = 0;
  let truncated = false;
  const byteLength = text => new TextEncoder().encode(text).length;
  function shape(item, depth) {
    nodes += 1;
    if (nodes > MAX_NODES) { truncated = true; return { type: 'truncated' }; }
    const type = typeName(item);
    if (type === 'string') return { type, bytes: byteLength(item) };
    if (type !== 'object' && type !== 'array') return { type };
    if (depth >= MAX_DEPTH) { truncated = true; return { type, truncated: true }; }
    if (Array.isArray(item)) {
      const items = [];
      const count = Math.min(item.length, MAX_ARRAY);
      if (count < item.length) truncated = true;
      for (let i = 0; i < count && nodes < MAX_NODES; i += 1) items.push(shape(item[i], depth + 1));
      if (items.length < item.length) truncated = true;
      return { type, length: item.length, items };
    }
    const keys = Object.keys(item);
    const values = [];
    for (const key of keys.slice(0, MAX_NODES)) {
      if (nodes >= MAX_NODES) { truncated = true; break; }
      values.push(shape(item[key], depth + 1));
    }
    if (values.length < keys.length) truncated = true;
    return { type, propertyCount: keys.length, values };
  }
  const result = shape(value, 0);
  if (truncated) result.truncated = true;
  return result;
}

export function shapeJsonText(text) {
  const bytes = new TextEncoder().encode(text).length;
  if (bytes > MAX_BODY_BYTES) return { type: 'opaque', bytes, truncated: true };
  try { return shapeJsonValue(JSON.parse(text)); }
  catch { return { type: 'opaque', bytes }; }
}

export function sanitizeContentType(headers) {
  if (!headers || typeof headers !== 'object') return undefined;
  const entries = Array.isArray(headers)
    ? headers.map(header => [header?.name, header?.value])
    : Object.entries(headers);
  for (const [name, rawValue] of entries) {
    if (String(name).toLowerCase() !== 'content-type') continue;
    const value = String(rawValue ?? '').trim().toLowerCase();
    const media = value.split(';', 1)[0].trim();
    if (media === 'application/json' || media === 'application/json+protobuf' || media === 'application/x-protobuf' || media === 'application/protobuf') return media;
    return undefined;
  }
  return undefined;
}

export function makeRecord({ phase, rpc, httpMethod, status, contentType, body }) {
  if (!rpc || !['request', 'response'].includes(phase)) return null;
  const record = { type: phase, service: rpc.service, method: rpc.method };
  if (['POST', 'GET', 'OPTIONS'].includes(httpMethod)) record.httpMethod = httpMethod;
  if (Number.isInteger(status)) record.status = status;
  if (['application/json', 'application/json+protobuf', 'application/x-protobuf', 'application/protobuf'].includes(contentType)) record.contentType = contentType;
  if (body) record.body = body;
  return record;
}
