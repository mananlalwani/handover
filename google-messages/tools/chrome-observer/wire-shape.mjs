// Structural protobuf facts only. No field values or speculative submessage decoding.
// Wire format: https://protobuf.dev/programming-guides/encoding/
export function protobufShape(bytes) {
  if (!(bytes instanceof Uint8Array)) return { type: 'unavailable' };
  const size = bytes.length;
  if (size > 1024 * 1024) return { type: 'opaque', bytes: size, truncated: true };
  let offset = 0;
  const fields = [];
  function varint() {
    let value = 0n;
    for (let i = 0; i < 10; i += 1) {
      if (offset >= size) throw new Error('invalid');
      const b = bytes[offset++];
      if (i === 9 && b > 1) throw new Error('invalid');
      value |= BigInt(b & 127) << BigInt(7 * i);
      if (b < 128) return value;
    }
    throw new Error('invalid');
  }
  try {
    while (offset < size) {
      if (fields.length >= 128) return { type: 'protobuf_wire', bytes: size, fields, truncated: true };
      const tag = varint();
      const number = tag >> 3n;
      const wire = Number(tag & 7n);
      if (number < 1n || number > 536870911n) throw new Error('invalid');
      const field = { field: Number(number), wireType: wire };
      if (wire === 0) varint();
      else if (wire === 1) offset += 8;
      else if (wire === 5) offset += 4;
      else if (wire === 2) {
        const length = varint();
        if (length > BigInt(size - offset)) throw new Error('invalid');
        field.bytes = Number(length);
        offset += field.bytes;
      } else throw new Error('unsupported');
      if (offset > size) throw new Error('invalid');
      fields.push(field);
    }
    return { type: 'protobuf_wire', bytes: size, fields };
  } catch {
    return { type: 'opaque', bytes: size };
  }
}
