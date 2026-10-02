const RPC_RE = /^\/\$rpc\/google\.internal\.communications\.instantmessaging\.v1\.([A-Za-z0-9_]+)\/([A-Za-z0-9_]+)$/;
const MAX_BODY_BYTES = 1024 * 1024;
const MAX_NODES = 512;
const MAX_DEPTH = 6;
const MAX_ARRAY = 128;

export function matchRpcUrl(input) {
  try {
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
    const url = new URL(input);
    return url.protocol === 'https:' && url.hostname.toLowerCase() === 'messages.google.com' &&
      !url.username && !url.password && !url.port;
  } catch { return false; }
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
