import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

const share = vm.createContext({});
vm.runInContext(fs.readFileSync(new URL('../Share.js', import.meta.url), 'utf8').replace('.pragma library', ''), share);
const begin = () => share.start('phone', 'share.file');
const accept = state => share.accepted(state, 'share.file', 'phone', 'transfer');
const progress = { device_id: 'phone', transfer_id: 'transfer', bytes_sent: 10, total_bytes: 10 };
const completed = { device_id: 'phone', transfer_id: 'transfer', status: 'completed' };

test('picker only offers connected paired devices advertising file transfer', () => {
    const device = { connected: true, paired: true, capabilities: ['file_transfer'] };
    assert.equal(share.devices([device, { ...device, connected: false }, { ...device, paired: false },
        { ...device, capabilities: [] }, { connected: true, paired: true }]).length, 1);
});
test('paths preserve special characters and URL resources keep their meaning', () => {
    const path = '/tmp/a #?%\n✓.txt';
    assert.equal(new URL(share.resource(path).value).pathname, '/tmp/a%20%23%3F%25%0A%E2%9C%93.txt');
    assert.equal(share.resource('https://example.com/a?q=1#x').method, 'share.url');
    assert.equal(share.resource('file:///tmp/a%20b').value, 'file:///tmp/a%20b');
    for (const input of ['', 'relative/path', 'javascript:alert(1)', 'content://file'])
        assert.equal(share.resource(input), null);
});
test('acceptance and full sender progress never claim completion', () => {
    let state = accept(begin());
    assert.equal(state.phase, 'accepted');
    assert.match(state.text, /not confirmed/);
    state = share.event(state, progress, false);
    assert.equal(state.phase, 'accepted');
    assert.match(state.text, /Waiting for completion/);
    assert.equal(share.event(state, completed, true).phase, 'done');
});
test('unrelated events and replies cannot complete a transfer', () => {
    const state = accept(begin());
    assert.equal(share.event(state, { ...completed, device_id: 'other' }, true), state);
    assert.equal(share.event(state, { ...completed, transfer_id: 'other' }, true), state);
    const pending = begin();
    assert.equal(share.accepted(pending, 'share.url', 'phone', 'transfer'), pending);
    assert.equal(share.accepted(pending, 'share.file', 'other', 'transfer'), pending);
});
test('completion before acceptance is retained and cannot be overwritten by late progress', () => {
    let state = share.event(begin(), completed, true);
    state = share.event(state, progress, false);
    state = accept(state);
    assert.equal(state.phase, 'done');
    assert.equal(state.text, 'Transfer completed.');
    assert.equal(share.event(state, progress, false), state);
});
test('early events stay bounded and match the accepted ID', () => {
    let state = begin();
    for (let index = 0; index < 100; index++)
        state = share.event(state, { ...progress, transfer_id: String(index) }, false);
    assert.equal(state.early.length, 32);
    assert.match(accept(state).text, /not confirmed/);
});
test('missing transfer IDs leave completion unconfirmed', () => {
    const state = share.accepted(begin(), 'share.file', 'phone', '');
    assert.equal(share.event(state, completed, true), state);
    assert.equal(state.phase, 'accepted');
});
test('failure and disconnect report evidence without replaying commands', () => {
    assert.match(share.failed(begin(), 'rejected').text, /rejected/);
    assert.match(share.event(accept(begin()), { ...completed, status: 'failed', reason: 'storage' }, true).text, /storage/);
    const state = share.disconnected(accept(begin()));
    assert.equal(state.phase, 'done');
    assert.match(state.text, /outcome is unknown/);
    assert.equal(share.event(state, completed, true), state);
});

test('service preserves transfer IDs while keeping existing acceptance notification', () => {
    const source = fs.readFileSync(new URL('../HandoverService.qml', import.meta.url), 'utf8');
    const start = source.indexOf('    function handleLine(');
    const end = source.indexOf('\n    Timer {', start);
    const calls = [];
    const service = vm.createContext({ protocolVersion: 1, maxLineBytes: 2 * 1024 * 1024,
        maxSnapshotBytes: 16 * 1024 * 1024, maxSnapshotChunks: 256,
        snapshotBuffer: null, snapshotKind: '', snapshotBytes: 0, snapshotChunks: 0,
        listAssembly: null,
        pendingCommand: { deviceId: 'phone', method: 'share.file' },
        shareAccepted: (...args) => calls.push(args), finishPending: (...args) => calls.push(args) });
    const helpers = ['utf8ByteLength'].map(name => {
        const helperStart = source.indexOf(`    function ${name}(`);
        const body = source.indexOf('{', helperStart);
        let depth = 1, helperEnd = body + 1;
        while (depth > 0) {
            if (source[helperEnd] === '{') depth++;
            if (source[helperEnd] === '}') depth--;
            helperEnd++;
        }
        return source.slice(helperStart, helperEnd);
    }).join('\n');
    vm.runInContext(helpers + '\n' + source.slice(start, end), service);
    service.handleLine(JSON.stringify({ protocol: 1, type: 'share_accepted', device_id: 'other', transfer_id: 'wrong' }));
    assert.equal(calls.length, 0);
    service.handleLine(JSON.stringify({ protocol: 1, type: 'share_accepted', device_id: 'phone', transfer_id: 'transfer' }));
    assert.deepEqual(calls, [['share.file', 'phone', 'transfer'], [true, '']]);
});
