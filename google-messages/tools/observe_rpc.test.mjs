import test from 'node:test';
import assert from 'node:assert/strict';
import { createHash } from 'node:crypto';
import { spawn } from 'node:child_process';
import { once } from 'node:events';
import { createServer } from 'node:net';
import { mkdtemp, readFile, rm, stat, writeFile } from 'node:fs/promises';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { bodyShape, makeRpcRecord, matchRpcUrl, validateDevToolsAddress } from './observe_rpc.mjs';

test('DevTools address accepts only loopback browser endpoint file contents', () => {
  assert.equal(validateDevToolsAddress('9222\n/devtools/browser/abc-123\n'), 'ws://127.0.0.1:9222/devtools/browser/abc-123');
  for (const bad of [
    '0\n/devtools/browser/id',
    '65536\n/devtools/browser/id',
    '9222\n/devtools/page/id',
    '9222\n/devtools/browser/id?x=1',
    '9222\nwss://evil.example/devtools/browser/id',
    '9222\n/devtools/browser/a/../../x',
    '127.0.0.1:9222\n/devtools/browser/id',
    `9222\n/devtools/browser/${'x'.repeat(5000)}`,
  ]) assert.throws(() => validateDevToolsAddress(bad));
});

test('RPC URL matcher enforces HTTPS, host allowlist, and exact path shape', () => {
  assert.deepEqual(matchRpcUrl('https://messages.google.com/$rpc/google.internal.communications.instantmessaging.v1.Foo/Send?token=secret#x'), { service: 'Foo', method: 'Send' });
  assert.deepEqual(matchRpcUrl('https://x.googleapis.com/$rpc/google.internal.communications.instantmessaging.v1.Foo/Get'), { service: 'Foo', method: 'Get' });
  for (const url of [
    'http://messages.google.com/$rpc/google.internal.communications.instantmessaging.v1.Foo/Send',
    'https://messages.google.com.evil.test/$rpc/google.internal.communications.instantmessaging.v1.Foo/Send',
    'https://googleapis.com.evil.test/$rpc/google.internal.communications.instantmessaging.v1.Foo/Send',
    'https://messages.google.com:444/$rpc/google.internal.communications.instantmessaging.v1.Foo/Send',
    'https://user:pass@messages.google.com/$rpc/google.internal.communications.instantmessaging.v1.Foo/Send',
    'https://evil.test/$rpc/google.internal.communications.instantmessaging.v1.Foo/Send',
    'https://messages.google.com/$rpc/google.internal.communications.instantmessaging.v1.Foo/Send/extra',
  ]) assert.equal(matchRpcUrl(url), null);
});

test('shape contains structure and string byte lengths but no keys or values', () => {
  const shape = bodyShape(JSON.stringify({ secretKey: 'secret-value-π', nested: [true, 7] }));
  const serialized = JSON.stringify(shape);
  assert.equal(shape.propertyCount, 2);
  assert.equal(shape.values[0].bytes, Buffer.byteLength('secret-value-π'));
  assert.equal(serialized.includes('secretKey'), false);
  assert.equal(serialized.includes('secret-value'), false);
});

test('shape limits nesting, array positions, and total nodes', () => {
  const deep = bodyShape(JSON.stringify({ a: { b: { c: { d: { e: { f: { g: 1 } } } } } } }));
  assert.equal(deep.truncated, true);
  const many = bodyShape(JSON.stringify(Array.from({ length: 200 }, (_, i) => i)));
  assert.equal(many.items.length, 128);
  assert.equal(many.truncated, true);
  const tooManyNodes = bodyShape(JSON.stringify(Array.from({ length: 512 }, () => [0])));
  assert.equal(tooManyNodes.truncated, true);
  const tooLarge = bodyShape('x'.repeat(1024 * 1024 + 1));
  assert.equal(tooLarge.truncated, true);
});

test('records include only normalized fields and omit URL query, fragments, and secrets', () => {
  const record = makeRpcRecord({
    url: 'https://messages.google.com/$rpc/google.internal.communications.instantmessaging.v1.Chat/Send?auth=supersecret#fragment',
    method: 'DELETE', status: 200, contentType: 'application/json', phase: 'response',
  });
  const serialized = JSON.stringify(record);
  assert.equal(record.httpMethod, undefined);
  assert.equal(serialized.includes('supersecret'), false);
  assert.equal(serialized.includes('fragment'), false);
  assert.equal(record.service, 'Chat');
});

test('CLI observes matching RPC facts through localhost CDP with bounded, private output', { timeout: 12000 }, async () => {
  const rpcUrl = 'https://messages.google.com/$rpc/google.internal.communications.instantmessaging.v1.Chat/Send?auth=QUERY_SECRET#fragment';
  const responseBody = Buffer.from(JSON.stringify({ responseSecretKey: 'RESPONSE_SECRET_VALUE', ok: true })).toString('base64');
  const commands = [];
  const server = createServer(socket => {
    let buffer = Buffer.alloc(0);
    let upgraded = false;
    const send = payload => {
      const data = Buffer.from(JSON.stringify(payload));
      const header = data.length < 126 ? Buffer.from([0x81, data.length]) : Buffer.from([0x81, 126, data.length >> 8, data.length & 0xff]);
      socket.write(Buffer.concat([header, data]));
    };
    socket.on('data', chunk => {
      buffer = Buffer.concat([buffer, chunk]);
      if (!upgraded) {
        const end = buffer.indexOf('\r\n\r\n');
        if (end < 0) return;
        const headers = buffer.subarray(0, end).toString();
        const key = headers.match(/Sec-WebSocket-Key: (.+)\r/i)?.[1];
        if (!key) { socket.destroy(); return; }
        const accept = createHash('sha1').update(`${key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11`).digest('base64');
        socket.write(`HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: ${accept}\r\n\r\n`);
        buffer = buffer.subarray(end + 4);
        upgraded = true;
      }
      while (upgraded && buffer.length >= 6) {
        const lengthByte = buffer[1] & 0x7f;
        let headerLength = 2;
        let length = lengthByte;
        if (lengthByte === 126) { if (buffer.length < 8) return; length = buffer.readUInt16BE(2); headerLength = 4; }
        else if (lengthByte === 127) { if (buffer.length < 14) return; length = Number(buffer.readBigUInt64BE(2)); headerLength = 10; }
        const masked = (buffer[1] & 0x80) !== 0;
        const maskOffset = headerLength;
        if (masked) headerLength += 4;
        if (buffer.length < headerLength + length) return;
        const payload = Buffer.from(buffer.subarray(headerLength, headerLength + length));
        if (masked) for (let i = 0; i < payload.length; i += 1) payload[i] ^= buffer[maskOffset + (i % 4)];
        const opcode = buffer[0] & 0x0f;
        buffer = buffer.subarray(headerLength + length);
        if (opcode === 8) { socket.end(); return; }
        if (opcode !== 1) continue;
        const message = JSON.parse(payload.toString('utf8'));
        commands.push(message.method);
        const reply = (result = {}) => send({ id: message.id, result });
        if (message.method === 'Target.setDiscoverTargets') reply();
        else if (message.method === 'Target.getTargets') reply({ targetInfos: [{ targetId: 'page-1', type: 'page', url: 'https://messages.google.com/' }] });
        else if (message.method === 'Target.attachToTarget') reply({ sessionId: 'session-1' });
        else if (message.method === 'Network.enable') {
          reply();
          setImmediate(() => {
            send({ method: 'Network.requestWillBeSent', sessionId: 'session-1', params: { requestId: 'request-1', request: {
              url: rpcUrl, method: 'POST', headers: { 'Content-Type': 'application/json', Authorization: 'HEADER_SECRET' },
              postData: JSON.stringify({ requestSecretKey: 'REQUEST_SECRET_VALUE', text: 'private text' }),
            } } });
            send({ method: 'Network.responseReceived', sessionId: 'session-1', params: { requestId: 'request-1', response: {
              status: 200, headers: { 'Content-Type': 'application/json', Cookie: 'COOKIE_SECRET' },
            } } });
            send({ method: 'Network.loadingFinished', sessionId: 'session-1', params: { requestId: 'request-1' } });
          });
        } else if (message.method === 'Network.getResponseBody') reply({ body: responseBody, base64Encoded: true });
        else send({ id: message.id, error: { message: 'unexpected command' } });
      }
    });
  });
  const temp = await mkdtemp(join(tmpdir(), 'observe-rpc-test-'));
  let child;
  try {
    server.listen(0, '127.0.0.1');
    await once(server, 'listening');
    const { port } = server.address();
    const devtoolsFile = join(temp, 'DevToolsActivePort');
    const outputFile = join(temp, 'capture.jsonl');
    await writeFile(devtoolsFile, `${port}\n/devtools/browser/test-id\n`, { mode: 0o600 });
    const startedAt = Date.now();
    child = spawn(process.execPath, [new URL('./observe_rpc.mjs', import.meta.url).pathname,
      '--devtools-file', devtoolsFile, '--output', outputFile, '--duration', '1'], { stdio: ['ignore', 'pipe', 'pipe'] });
    let stdout = '';
    let stderr = '';
    child.stdout.setEncoding('utf8').on('data', chunk => { stdout += chunk; });
    child.stderr.setEncoding('utf8').on('data', chunk => { stderr += chunk; });
    const [code] = await once(child, 'exit');
    const elapsed = Date.now() - startedAt;
    assert.equal(code, 0, stderr);
    assert.ok(elapsed >= 900 && elapsed < 7000, `duration exit took ${elapsed}ms; commands=${commands.join(',')}; stdout=${stdout}; stderr=${stderr}`);
    const outputStat = await stat(outputFile);
    assert.equal(outputStat.mode & 0o777, 0o600);
    const lines = (await readFile(outputFile, 'utf8')).trim().split('\n').map(line => JSON.parse(line));
    assert.deepEqual(lines.map(line => line.type), ['request', 'response', 'response']);
    assert.deepEqual(lines.map(line => [line.service, line.method]), Array(3).fill(['Chat', 'Send']));
    assert.equal(lines[0].httpMethod, 'POST');
    assert.equal(lines[1].status, 200);
    assert.equal(lines[2].body.propertyCount, 2);
    assert.deepEqual(commands.sort(), ['Network.enable', 'Network.getResponseBody', 'Target.attachToTarget', 'Target.getTargets', 'Target.setDiscoverTargets'].sort());
    const allOutput = `${stdout}\n${await readFile(outputFile, 'utf8')}`;
    for (const secret of ['QUERY_SECRET', 'HEADER_SECRET', 'COOKIE_SECRET', 'requestSecretKey', 'REQUEST_SECRET_VALUE', 'responseSecretKey', 'RESPONSE_SECRET_VALUE', 'private text', 'fragment']) {
      assert.equal(allOutput.includes(secret), false, `output leaked ${secret}`);
    }
    assert.equal(stdout.includes(outputFile), true);
  } finally {
    if (child && child.exitCode === null) child.kill('SIGKILL');
    server.close();
    await rm(temp, { recursive: true, force: true });
  }
});
