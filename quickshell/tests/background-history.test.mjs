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
        automaticHistoryNext: null, conversationMessages: {}, conversationCursors: {},
        sendRequest: (type, fields) => { requests.push({type, fields}); return true; },
        historyProgress: () => {}, automaticHistoryPageTimer: { restart() {} },
        console, lastError: '',
    });
    vm.runInContext(['sameConversationId','conversationKey','loadHistory','loadAllHistory','setHistoryFocus','isLoadingAllHistory','replaceMessages','handleLine'].map(extract).join('\n'), context);
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
