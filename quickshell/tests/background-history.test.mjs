import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';
function extract(name) {
    const source = fs.readFileSync(new URL('../HandoverService.qml', import.meta.url), 'utf8');
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
const thread = { account_id: 'fixture', local_id: 'thread' };
function service() {
    const requests = [];
    const context = vm.createContext({
        protocolVersion: 1, historyRequests: {}, exhaustiveHistoryLoads: {}, historyFocus: null,
        messagingAccounts: [], conversations: [], outgoingOperations: [],
        automaticHistoryNext: null, conversationMessages: {}, conversationCursors: {},
        sendRequest: (type, fields) => { requests.push({type, fields}); return true; },
        historyProgress: () => {}, automaticHistoryPageTimer: { restart() {} },
        console, lastError: '', finishPending() {}, nativePending: false, pendingMessaging: null,
    });
    vm.runInContext(['sameConversationId','conversationKey','dropAccountEntries','stopHistoryLoad','loadHistory','loadAllHistory','setHistoryFocus','isLoadingAllHistory','replaceMessages','handleLine'].map(extract).join('\n'), context);
    return { context, requests };
}
test('automatic loading waits for the current page, then stops when focus moves', () => {
    const { context: c, requests } = service();
    c.setHistoryFocus(thread);
    assert.equal(c.loadHistory(thread, 20), true);
    assert.equal(c.loadAllHistory(thread, true), false);
    c.handleLine(JSON.stringify({protocol:1,type:'history',conversation_id:thread,messages:[],cursor_next:'older'}));
    assert.equal(c.loadAllHistory(thread, true), true);
    c.handleLine(JSON.stringify({protocol:1,type:'history',conversation_id:thread,messages:[],cursor_next:'older'}));
    assert.equal(requests.length, 2);
    assert.equal(c.automaticHistoryNext.cursor, 'older');
    c.setHistoryFocus({...thread,local_id:'different'});
    assert.equal(c.isLoadingAllHistory(thread), false);
    assert.equal(c.automaticHistoryNext, null);
});
test('an explicit full-history request continues after switching conversations', () => {
    const {context:c,requests} = service();
    c.setHistoryFocus(thread);
    c.loadAllHistory(thread, false);
    c.setHistoryFocus(null);
    c.handleLine(JSON.stringify({protocol:1,type:'history',conversation_id:thread,messages:[],cursor_next:'older'}));
    assert.equal(requests.length, 2);
    assert.equal(requests[1].fields.cursor, 'older');
    c.handleLine(JSON.stringify({protocol:1,type:'history',conversation_id:thread,messages:[],cursor_next:null}));
    assert.equal(c.isLoadingAllHistory(thread), false);
});
test('a repeated provider cursor stops loading instead of downloading forever', () => {
    const {context:c,requests} = service();
    c.loadAllHistory(thread, false);
    const response = JSON.stringify({protocol:1,type:'history',conversation_id:thread,messages:[],cursor_next:'same'});
    c.handleLine(response);
    c.handleLine(response);
    assert.equal(c.isLoadingAllHistory(thread), false);
    assert.equal(requests.length, 2);
});

test('an unrelated command error does not interrupt a history load', () => {
    const {context:c,requests} = service();
    c.loadAllHistory(thread, false);
    c.handleLine(JSON.stringify({protocol:1,type:'error',code:'backend_rejected',message:'command rejected'}));
    assert.equal(c.isLoadingAllHistory(thread), true);
    c.handleLine(JSON.stringify({protocol:1,type:'history',conversation_id:thread,messages:[],cursor_next:'older'}));
    assert.equal(requests.length, 2);
    assert.equal(requests[1].fields.cursor, 'older');
    c.handleLine(JSON.stringify({protocol:1,type:'error',code:'history_unavailable',message:'history unavailable'}));
    assert.equal(c.isLoadingAllHistory(thread), true);
    const other = {...thread, local_id: 'other'};
    c.handleLine(JSON.stringify({protocol:1,type:'error',code:'messaging_unavailable',message:'helper down',conversation_id:other}));
    assert.equal(c.isLoadingAllHistory(thread), true);
    c.handleLine(JSON.stringify({protocol:1,type:'error',code:'history_unavailable',message:'history unavailable',conversation_id:thread}));
    assert.equal(c.isLoadingAllHistory(thread), false);
    assert.equal(Object.keys(c.historyRequests).length, 0);
});

test('removing an account drops that account\'s cached threads', () => {
    const {context:c} = service();
    const other = { account_id: 'other', local_id: 'thread' };
    c.conversationMessages = { 'fixture:thread': [{id:{conversation_id:thread,local_id:'m'}}], 'other:thread': [{id:{conversation_id:other,local_id:'n'}}] };
    c.conversationCursors = { 'fixture:thread': 'older', 'other:thread': 'keep' };
    c.historyRequests = { 'fixture:thread': true };
    c.exhaustiveHistoryLoads = { 'fixture:thread': { automatic: true } };
    c.messagingAccounts = [{id:'fixture'}, {id:'other'}];
    c.conversations = [{id:thread}, {id:other}];
    c.outgoingOperations = [{conversation_id:thread}, {conversation_id:other}];
    c.handleLine(JSON.stringify({protocol:1,type:'account_removed',account_id:'fixture'}));
    assert.deepEqual(Object.keys(c.conversationMessages), ['other:thread']);
    assert.deepEqual(Object.keys(c.conversationCursors), ['other:thread']);
    assert.equal(c.conversationCursors['other:thread'], 'keep');
    assert.deepEqual(Object.keys(c.historyRequests), []);
    assert.deepEqual(Object.keys(c.exhaustiveHistoryLoads), []);
    assert.deepEqual(c.messagingAccounts.map(account => account.id), ['other']);
    assert.equal(c.conversations.length, 1);
    assert.equal(c.conversations[0].id.local_id, 'thread');
    assert.equal(c.conversations[0].id.account_id, 'other');
    assert.equal(c.outgoingOperations.length, 1);
    assert.equal(c.outgoingOperations[0].conversation_id.account_id, 'other');
});
