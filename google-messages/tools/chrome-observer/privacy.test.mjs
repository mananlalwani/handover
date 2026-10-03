import test from 'node:test';
import assert from 'node:assert/strict';
import { browserLookupRequest, eligibleTabUrl, makeRecord, matchGaiaProbeUrl, matchRpcUrl, sanitizeContentType, shapeJsonText } from './privacy.mjs';

test('browser comparison accepts only bounded token-free read-only lookup bodies', () => {
  const body = [['12345678-1234-4234-8234-123456789abc', null, 'GDitto', null, null, null, [null, null, 20261001, 2, 0, null, 4, null, 6]], [[3, 'messages-web-0123456789abcdef0123456789abcdef']], 1, 'GDitto'];
  assert.deepEqual(browserLookupRequest(JSON.stringify(body)), body);
  for (const mutate of [
    b => { b[2] = 0; },
    b => { b[0][5] = 'PRIVATE_TOKEN'; },
    b => { b[0][1] = 'PRIVATE_ACCOUNT'; },
    b => { b[1][0][1] = 'PRIVATE_DEVICE'; },
    b => { b[0][6][0] = 'PRIVATE_FIELD'; },
    b => { b.push('PRIVATE_EXTRA'); },
  ]) {
    const invalid = structuredClone(body); mutate(invalid);
    assert.equal(browserLookupRequest(JSON.stringify(invalid)), null);
  }
  assert.equal(browserLookupRequest('x'.repeat(2049)), null);
  assert.equal(browserLookupRequest('{}'), null);
});

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

test('Gaia probe accepts only the three exact HTTPS service origins and registration path', () => {
  const path = '/$rpc/google.internal.communications.instantmessaging.v1.Registration/SignInGaia';
  for (const host of ['instantmessaging-pa.googleapis.com', 'instantmessaging-pa.clients6.google.com', 'instantmessaging-pa-jms-us.clients6.google.com']) {
    assert.equal(matchGaiaProbeUrl(`https://${host}${path}?token=SECRET`), `https://${host}`);
  }
  for (const url of [
    `https://evil.instantmessaging-pa.googleapis.com${path}`,
    `https://instantmessaging-pa.googleapis.com.evil.test${path}`,
    `https://instantmessaging-pa.googleapis.com:444${path}`,
    `http://instantmessaging-pa.googleapis.com${path}`,
    `https://instantmessaging-pa.googleapis.com${path}/extra`,
    `https://user:pass@instantmessaging-pa.googleapis.com${path}`,
  ]) assert.equal(matchGaiaProbeUrl(url), null);
});
