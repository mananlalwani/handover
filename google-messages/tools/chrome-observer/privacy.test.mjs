import test from 'node:test';
import assert from 'node:assert/strict';
import { eligibleTabUrl, makeRecord, matchRpcUrl, sanitizeContentType, shapeJsonText } from './privacy.mjs';

test('RPC and active tab URL validation reject deceptive origins and ports', () => {
  assert.deepEqual(matchRpcUrl('https://messages.google.com/$rpc/google.internal.communications.instantmessaging.v1.Chat/Send?secret=1'), { service: 'Chat', method: 'Send' });
  assert.deepEqual(matchRpcUrl('https://x.googleapis.com/$rpc/google.internal.communications.instantmessaging.v1.Chat/Get'), { service: 'Chat', method: 'Get' });
  for (const url of [
    'http://messages.google.com/$rpc/google.internal.communications.instantmessaging.v1.Chat/Send',
    'https://messages.google.com:444/$rpc/google.internal.communications.instantmessaging.v1.Chat/Send',
    'https://user:pass@messages.google.com/$rpc/google.internal.communications.instantmessaging.v1.Chat/Send',
    'https://messages.google.com.evil.test/$rpc/google.internal.communications.instantmessaging.v1.Chat/Send',
  ]) assert.equal(matchRpcUrl(url), null);
  assert.equal(eligibleTabUrl('https://messages.google.com/'), true);
  assert.equal(eligibleTabUrl('https://messages.google.com:444/'), false);
  assert.equal(eligibleTabUrl('https://messages.google.com.evil.test/'), false);
});

test('JSON shape keeps only types, property counts, positions, and string byte lengths', () => {
  const shape = shapeJsonText(JSON.stringify({ SECRET_KEY: 'private-π', list: [null, true] }));
  const output = JSON.stringify(shape);
  assert.equal(shape.propertyCount, 2);
  assert.equal(shape.values[0].bytes, new TextEncoder().encode('private-π').length);
  assert.equal(output.includes('SECRET_KEY'), false);
  assert.equal(output.includes('private-'), false);
  const tooLarge = shapeJsonText('x'.repeat(1024 * 1024 + 1));
  assert.equal(tooLarge.type, 'opaque');
  assert.equal(tooLarge.truncated, true);
  const manyItems = shapeJsonText(JSON.stringify(Array.from({ length: 200 }, (_, index) => index)));
  assert.equal(manyItems.items.length, 128);
  assert.equal(manyItems.truncated, true);
});

test('content type and records keep an explicit safe allowlist', () => {
  assert.equal(sanitizeContentType({ 'Content-Type': 'application/json+protobuf; charset=utf-8' }), 'application/json+protobuf');
  assert.equal(sanitizeContentType([{ name: 'Content-Type', value: 'text/plain' }]), undefined);
  const record = makeRecord({ phase: 'request', rpc: { service: 'Chat', method: 'Send' }, httpMethod: 'DELETE', contentType: 'text/plain' });
  assert.equal(JSON.stringify(record).includes('DELETE'), false);
  assert.equal(JSON.stringify(record).includes('text/plain'), false);
});
