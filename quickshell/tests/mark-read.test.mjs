import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

const source = fs.readFileSync(new URL('../HandoverService.qml', import.meta.url), 'utf8');
function functionSource(name) {
    const start = source.indexOf(`    function ${name}(`);
    assert.ok(start >= 0);
    const body = source.indexOf('{', start);
    let depth = 1;
    let end = body + 1;
    while (depth > 0 && end < source.length) {
        if (source[end] === '{') depth++;
        if (source[end] === '}') depth--;
        end++;
    }
    assert.equal(depth, 0);
    return source.slice(start, end);
}
function service(connected, capabilities) {
    const calls = [];
    const id = { account_id: 'account', local_id: 'thread' };
    const context = vm.createContext({
        messagingAccounts: [{ id: 'account', connected, authenticated: true }],
        conversations: [{ id, capabilities }],
        pendingMessaging: null,
        lastError: '',
        sameConversationId: (first, second) => first.account_id === second.account_id && first.local_id === second.local_id,
        sendRequest: (method, fields) => { calls.push({ method, fields }); return true; }
    });
    vm.runInContext(['sendMessaging', 'markRead', 'sendText'].map(functionSource).join('\n'), context);
    return { context, calls, id };
}
test('offline conversation switching does not occupy the send lock', () => {
    const { context, calls, id } = service(false, ['text', 'read_receipts']);
    assert.equal(context.markRead(id), false);
    assert.equal(context.pendingMessaging, null);
    assert.equal(calls.length, 0);
    assert.equal(context.sendText(id, 'new message', null), true);
    assert.equal(context.pendingMessaging.method, 'messages.send');
});
test('unsupported mark-read does not block composing on a native text account', () => {
    const { context, calls, id } = service(true, ['text']);
    assert.equal(context.markRead(id), false);
    assert.equal(context.pendingMessaging, null);
    assert.equal(calls.length, 0);
    assert.equal(context.sendText(id, 'new message', null), true);
});
test('online supported read command retains its acknowledged command flow', () => {
    const { context, calls, id } = service(true, ['text', 'read_receipts']);
    assert.equal(context.markRead(id), true);
    assert.equal(context.pendingMessaging.method, 'messages.read');
    assert.equal(calls.length, 1);
});
