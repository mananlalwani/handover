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

test('queued cancellation keeps the transfer live until the daemon reports a verdict', () => {
    const state = share.requestCancellation(accept(begin()));
    assert.equal(state.phase, 'accepted');
    assert.equal(state.cancelPending, true);
    assert.equal(share.canCancel(state), false);
    assert.match(state.text, /Requesting cancellation/);
    assert.equal(share.cancellationResult(state, 'other', 'transfer', true, ''), state);
    assert.equal(share.cancellationResult(state, 'phone', 'other', true, ''), state);
    const rejected = share.cancellationResult(state, 'phone', 'transfer', false, '');
    assert.equal(rejected.phase, 'accepted');
    assert.match(rejected.text, /could not be cancelled/);
    const cancelled = share.cancellationResult(state, 'phone', 'transfer', true, '');
    assert.equal(cancelled.phase, 'done');
    assert.equal(cancelled.text, 'Queued transfer cancelled.');
    assert.equal(share.canCancel(cancelled), false);
});

test('failed cancellation never reports a failed transfer or overwrites confirmed completion', () => {
    const pending = share.requestCancellation(accept(begin()));
    const failed = share.cancellationResult(pending, 'phone', 'transfer', false, 'backend unavailable');
    assert.equal(failed.phase, 'accepted');
    assert.match(failed.text, /Cancellation request failed/);
    const finished = share.event(pending, completed, true);
    assert.equal(share.cancellationResult(finished, 'phone', 'transfer', false, '').text, 'Transfer completed.');
    assert.match(share.disconnected(pending).text, /outcome is unknown/);
    assert.equal(share.canCancel(share.accepted(begin(), 'share.file', 'phone', '')), false);
});
