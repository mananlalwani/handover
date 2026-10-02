import test from 'node:test';
import assert from 'node:assert/strict';
import { protobufShape } from './wire-shape.mjs';

test('protobuf facts omit values and distinguish wire types without guessing payload semantics', () => {
  const secret = new TextEncoder().encode('private credential');
  const bytes = Uint8Array.from([8, 150, 1, 18, secret.length, ...secret, 29, 1, 2, 3, 4, 33, 1, 2, 3, 4, 5, 6, 7, 8]);
  assert.deepEqual(protobufShape(bytes), {
    type: 'protobuf_wire', bytes: bytes.length,
    fields: [{ field: 1, wireType: 0 }, { field: 2, wireType: 2, bytes: secret.length }, { field: 3, wireType: 5 }, { field: 4, wireType: 1 }],
  });
  assert.equal(JSON.stringify(protobufShape(bytes)).includes('private'), false);
});

test('malformed and unsupported encodings remain opaque without partial guessed schemas', () => {
  for (const values of [[0], [8], [18, 9, 1], [9, 1], [13, 1], [11], [8, ...Array(10).fill(255)], [128, 128, 128, 128, 16, 0]]) {
    assert.deepEqual(protobufShape(Uint8Array.from(values)), { type: 'opaque', bytes: values.length });
  }
});

test('protobuf observation bounds bytes and repeated field count', () => {
  assert.deepEqual(protobufShape(new Uint8Array(1024 * 1024 + 1)), { type: 'opaque', bytes: 1024 * 1024 + 1, truncated: true });
  const shape = protobufShape(Uint8Array.from(Array(129).fill([8, 1]).flat()));
  assert.equal(shape.truncated, true);
  assert.equal(shape.fields.length, 128);
});
