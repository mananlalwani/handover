import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

const MAX_LINE = 2 * 1024 * 1024;
const MAX_SNAPSHOT = 16 * 1024 * 1024;
const MAX_CHUNKS = 256;

function extract(name) {
    const source = fs.readFileSync(new URL('../HandoverService.qml', import.meta.url), 'utf8');
    const start = source.indexOf(`    function ${name}(`);
    assert.ok(start >= 0, `missing ${name}`);
    const body = source.indexOf('{', start);
    let depth = 1, end = body + 1;
    while (depth > 0) {
        if (source[end] === '{') depth++;
        if (source[end] === '}') depth--;
        end++;
    }
    return source.slice(start, end);
}

function service() {
    const reconnects = [];
    const c = vm.createContext({
        protocolVersion: 1,
        reconnects,
        maxLineBytes: MAX_LINE,
        maxSnapshotBytes: MAX_SNAPSHOT,
        maxSnapshotChunks: MAX_CHUNKS,
        snapshotBuffer: null, snapshotKind: '', snapshotBytes: 0, snapshotChunks: 0,
        listAssembly: null,
        outgoingOperations: [], devices: ['old-device'], notifications: ['old-notification'],
        mediaSessions: [], calls: [], messagingAccounts: [], conversations: [],
        typingStates: [], readStates: [], contacts: [], contactsChunkBuffer: null,
        audioStreams: [], pendingCommand: null, pendingMessaging: null,
        retryTimer: { restart() {} }, sharesInterrupted() {}, finishPending() {},
        finishMessaging() {}, refreshContacts() {}, refreshMessaging() {},
        lastError: '',
    });
    vm.runInContext([
        extract('clearSnapshotAssembly'), extract('utf8ByteLength'),
        extract('applySnapshot'),
        extract('scheduleReconnect').replace('function scheduleReconnect(', 'function realScheduleReconnect('),
        'function scheduleReconnect(message) { reconnects.push(message); realScheduleReconnect(message); }',
        extract('handleLine'),
    ].join('\n'), c);
    return { c, reconnects };
}

function chunk(type = 'subscribed_chunk', done = false) {
    return JSON.stringify({
        protocol: 1, type, done,
        devices: [], notifications: [], media_sessions: [], calls: [],
        messaging_accounts: [], conversations: [], typing_states: [], read_states: [],
    });
}

function paddedChunk(size, done = false) {
    const base = JSON.parse(chunk('subscribed_chunk', done));
    base.padding = '';
    const initial = JSON.stringify(base);
    base.padding = 'x'.repeat(size - initial.length);
    const line = JSON.stringify(base);
    assert.equal(Buffer.byteLength(line), size);
    return line;
}

function listChunk(type, done = false, padding = '') {
    const field = type === 'notifications_chunk' ? 'notifications' : 'media_sessions';
    return JSON.stringify({ protocol: 1, type, [field]: [], done, padding });
}

function paddedListChunk(size, type = 'notifications_chunk', done = false) {
    const initial = listChunk(type, done);
    const base = JSON.parse(initial);
    base.padding = 'x'.repeat(size - initial.length);
    const line = JSON.stringify(base);
    assert.equal(Buffer.byteLength(line), size);
    return line;
}

test('UTF-8 byte counting handles BMP and supplementary characters exactly', () => {
    const { c } = service();
    assert.equal(c.utf8ByteLength('ascii'), 5);
    assert.equal(c.utf8ByteLength('✓'), 3);
    assert.equal(c.utf8ByteLength('📱'), 4);
    assert.equal(c.utf8ByteLength('a✓📱'), 8);
});

test('a line at 2 MiB is accepted and the next byte reconnects without exposing payload', () => {
    const { c, reconnects } = service();
    c.handleLine(paddedChunk(MAX_LINE, true));
    assert.deepEqual(reconnects, []);
    c.handleLine(JSON.stringify({ protocol: 1, type: 'snapshot', secret: 'private' }) + ' '.repeat(MAX_LINE));
    assert.deepEqual(reconnects, ['daemon message exceeds protocol limit']);
    assert.ok(!reconnects[0].includes('private'));
});

test('snapshot sequences accept the byte boundary and reset accounting on completion', () => {
    const { c, reconnects } = service();
    for (let index = 0; index < 8; index++)
        c.handleLine(paddedChunk(MAX_LINE, index === 7));
    assert.deepEqual(reconnects, []);
    assert.equal(c.snapshotBuffer, null);
    assert.equal(c.snapshotBytes, 0);
    assert.equal(c.snapshotChunks, 0);
    assert.deepEqual(Array.from(c.devices), []);
});

test('snapshot aggregate overflow discards incomplete state and reconnects', () => {
    const { c, reconnects } = service();
    for (let index = 0; index < 8; index++)
        c.handleLine(paddedChunk(MAX_LINE, false));
    assert.notEqual(c.snapshotBuffer, null);
    c.handleLine(chunk());
    assert.deepEqual(reconnects, ['daemon snapshot exceeds protocol limit']);
    assert.equal(c.snapshotBuffer, null);
    assert.equal(c.snapshotBytes, 0);
});

test('the 256th chunk is accepted and the 257th reconnects', () => {
    const { c, reconnects } = service();
    for (let index = 0; index < MAX_CHUNKS; index++)
        c.handleLine(chunk());
    assert.deepEqual(reconnects, []);
    assert.equal(c.snapshotChunks, MAX_CHUNKS);
    c.handleLine(chunk());
    assert.deepEqual(reconnects, ['daemon snapshot exceeds protocol limit']);
});

test('mixed snapshot types and interleaved live events discard partial snapshots', () => {
    for (const intervening of [
        chunk('snapshot_chunk'),
        JSON.stringify({ protocol: 1, type: 'media', media_sessions: [] }),
        JSON.stringify({ protocol: 1, type: 'snapshot', devices: [] }),
    ]) {
        const { c, reconnects } = service();
        c.handleLine(chunk());
        c.handleLine(intervening);
        assert.equal(reconnects.length, 1);
        assert.equal(c.snapshotBuffer, null);
        assert.equal(c.snapshotKind, '');
    }
});

test('both chunk kinds publish only complete sequences and preserve old state until done', () => {
    for (const type of ['subscribed_chunk', 'snapshot_chunk']) {
        const { c, reconnects } = service();
        c.outgoingOperations = ['old-operation'];
        c.handleLine(chunk(type));
        assert.deepEqual(Array.from(c.devices), ['old-device']);
        assert.deepEqual(Array.from(c.outgoingOperations), ['old-operation']);
        const final = JSON.parse(chunk(type, true));
        final.devices = [{ id: 'new-device' }];
        c.handleLine(JSON.stringify(final));
        assert.deepEqual(Array.from(c.devices, device => device.id), ['new-device']);
        assert.deepEqual(reconnects, []);
        assert.equal(c.snapshotKind, '');
        assert.deepEqual(Array.from(c.outgoingOperations), []);
    }
});

test('disconnect clears incomplete snapshot bookkeeping', () => {
    const { c } = service();
    c.handleLine(chunk());
    c.scheduleReconnect('daemon disconnected');
    assert.equal(c.snapshotBuffer, null);
    assert.equal(c.snapshotKind, '');
    assert.equal(c.snapshotBytes, 0);
    assert.equal(c.snapshotChunks, 0);
    assert.equal(c.lastError, 'daemon disconnected');
});

test('valid JSON with the wrong record shape reconnects without exposing values', () => {
    for (const value of [null, 7, 'private-sentinel', [], {protocol: 1}]) {
        const { c, reconnects } = service();
        c.handleLine(JSON.stringify(value));
        assert.deepEqual(reconnects, ['daemon sent invalid message']);
    }
});

test('list chunks publish atomically and replace only their own collection', () => {
    const { c, reconnects } = service();
    c.notifications = [{ id: 'old-notification' }];
    c.mediaSessions = [{ id: 'old-media' }];
    c.handleLine(JSON.stringify({ protocol: 1, type: 'notifications_chunk', notifications: [{ id: 'new-notification' }], done: false }));
    assert.deepEqual(Array.from(c.notifications, item => item.id), ['old-notification']);
    assert.deepEqual(Array.from(c.mediaSessions, item => item.id), ['old-media']);
    c.handleLine(JSON.stringify({ protocol: 1, type: 'notifications_chunk', notifications: [{ id: 'another-notification' }], done: true }));
    assert.deepEqual(Array.from(c.notifications, item => item.id), ['new-notification', 'another-notification']);
    assert.deepEqual(Array.from(c.mediaSessions, item => item.id), ['old-media']);
    assert.equal(c.listAssembly, null);
    assert.deepEqual(reconnects, []);
});

test('list sequences reject kind changes and every interleaved message', () => {
    for (const intervening of [
        listChunk('media_chunk'),
        JSON.stringify({ protocol: 1, type: 'notifications', notifications: [] }),
        chunk(),
    ]) {
        const { c, reconnects } = service();
        c.handleLine(listChunk('notifications_chunk'));
        c.handleLine(intervening);
        assert.deepEqual(reconnects, ['daemon interrupted list sequence']);
        assert.equal(c.listAssembly, null);
        assert.deepEqual(Array.from(c.notifications), []);
    }
});

test('list sequences enforce aggregate byte and chunk limits and reset on disconnect', () => {
    const { c, reconnects } = service();
    for (let index = 0; index < 8; index++)
        c.handleLine(paddedListChunk(MAX_LINE, 'notifications_chunk'));
    assert.equal(c.listAssembly.bytes, MAX_SNAPSHOT);
    c.handleLine(listChunk('notifications_chunk'));
    assert.deepEqual(reconnects, ['daemon list exceeds protocol limit']);
    assert.equal(c.listAssembly, null);

    const next = service();
    for (let index = 0; index < MAX_CHUNKS; index++)
        next.c.handleLine(listChunk('media_chunk'));
    assert.deepEqual(next.reconnects, []);
    assert.equal(next.c.listAssembly.chunks, MAX_CHUNKS);
    next.c.handleLine(listChunk('media_chunk'));
    assert.deepEqual(next.reconnects, ['daemon list exceeds protocol limit']);
    assert.equal(next.c.listAssembly, null);

    const disconnect = service();
    disconnect.c.handleLine(listChunk('media_chunk'));
    disconnect.c.scheduleReconnect('daemon disconnected');
    assert.equal(disconnect.c.listAssembly, null);
});
