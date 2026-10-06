import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';

function extract(file, name) {
    const source = fs.readFileSync(new URL(file, import.meta.url), 'utf8');
    const start = source.indexOf(`    function ${name}(`);
    assert.ok(start >= 0);
    const body = source.indexOf('{', start);
    let depth = 1, end = body + 1;
    while (depth > 0) {
        if (source[end] === '{') depth++;
        if (source[end] === '}') depth--;
        end++;
    }
    return source.slice(start, end);
}
function labels(contacts = []) {
    const context = vm.createContext({ contacts });
    vm.runInContext(['contactForParticipant', 'contactLabel'].map(name => extract('../HandoverService.qml', name)).join('\n'), context);
    context.HandoverService = context;
    vm.runInContext(extract('../pages/MessagesPage.qml', 'conversationLabel'), context);
    return context;
}
const peer = { local_id: 'peer:fixture', address: '+15555550100', display_name: 'Fixture person', is_self: false };
test('direct phone-number title does not hide a saved participant name', () => {
    assert.equal(labels().conversationLabel({ kind: 'direct', title: '+1 (555) 555-0100', participants: [peer] }), 'Fixture person');
});
test('custom and group titles remain intact', () => {
    for (const [kind, title] of [['direct', 'Custom title'], ['group', '+1 (555) 555-0100']])
        assert.equal(labels().conversationLabel({ kind, title, participants: [peer] }), title);
});
test('self role takes precedence over a number or contact name', () => {
    assert.equal(labels().contactLabel({ ...peer, is_self: true }), 'You');
});
test('self-only numeric conversation is named You', () => {
    assert.equal(labels().conversationLabel({ kind: 'direct', title: '+15555550100', participants: [{ local_id: 'self:fixture', is_self: true }] }), 'You');
});

test('a locally formatted numeric title uses the sole named peer', () => {
    assert.equal(labels().conversationLabel({ kind: 'direct', title: '(555) 555-0100', participants: [peer] }), 'Fixture person');
});

test('a synced phone contact resolves a numeric Google participant and title', () => {
    const contact = { display_name: 'Phone contact', phones: ['+1 (555) 555-0100'], emails: [] };
    assert.equal(labels([contact]).conversationLabel({ kind: 'direct', title: '5555550100', participants: [{ ...peer, display_name: '+15555550100' }] }), 'Phone contact');
});
