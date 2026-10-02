import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

function eventHook() {
  const listeners = new Set();
  return {
    addListener(listener) { listeners.add(listener); },
    fire(...args) { for (const listener of listeners) listener(...args); },
    listener() { return [...listeners][0]; },
  };
}

function deferred() {
  let resolve;
  const promise = new Promise(done => { resolve = done; });
  return { promise, resolve };
}

test('worker validates the active tab, uses only Network CDP calls, and isolates stale body replies', async () => {
  const calls = [];
  const debuggerCommands = [];
  const bodyWait = deferred();
  const bodyPayload = text => ({ body: btoa(text), base64Encoded: true });
  const chrome = {
    runtime: { id: 'observer-test', onMessage: eventHook(), sendMessage: async () => {} },
    tabs: {
      active: { id: 17, url: 'https://evil.test/' },
      query: async () => [chrome.tabs.active],
      onUpdated: eventHook(), onRemoved: eventHook(),
    },
    alarms: { create: (...args) => calls.push(['alarm-create', ...args]), clear: (...args) => calls.push(['alarm-clear', ...args]), onAlarm: eventHook() },
    debugger: {
      onEvent: eventHook(), onDetach: eventHook(),
      attach: async (...args) => calls.push(['attach', ...args]),
      detach: async (...args) => calls.push(['detach', ...args]),
      sendCommand: async (source, method, params) => {
        debuggerCommands.push(method);
        calls.push([method, source, params]);
        if (method === 'Network.getResponseBody') {
          return calls.filter(call => call[0] === method).length === 1
            ? bodyWait.promise
            : bodyPayload(JSON.stringify({ secondSecret: 'NEW_RESPONSE_SECRET', ok: true }));
        }
        return {};
      },
    },
  };
  globalThis.chrome = chrome;
  const manifest = JSON.parse(await readFile(new URL('./manifest.json', import.meta.url), 'utf8'));
  assert.deepEqual(manifest.permissions.sort(), ['activeTab', 'alarms', 'debugger']);
  for (const forbidden of ['host_permissions', 'tabs', 'scripting', 'cookies', 'storage', 'downloads', 'nativeMessaging']) {
    assert.equal(Object.hasOwn(manifest, forbidden), false);
  }
  await import(`./service-worker.js?test=${Date.now()}`);

  async function send(payload, sender = { id: 'observer-test' }) {
    return new Promise(resolve => {
      const asyncReply = chrome.runtime.onMessage.listener()(payload, sender, resolve);
      if (asyncReply !== true) resolve(undefined);
    });
  }
  async function waitFor(predicate) {
    for (let i = 0; i < 100; i += 1) {
      if (await predicate()) return;
      await new Promise(resolve => setTimeout(resolve, 2));
    }
    assert.fail('worker did not reach expected CDP state');
  }
  function rpcEvents(id) {
    const url = `https://messages.google.com/$rpc/google.internal.communications.instantmessaging.v1.Chat/Send?query=URL_SECRET_${id}`;
    chrome.debugger.onEvent.fire({ tabId: 17 }, 'Network.requestWillBeSent', { requestId: id, request: {
      url, method: 'POST', headers: { 'Content-Type': 'application/json', Authorization: 'HEADER_SECRET' },
      postData: JSON.stringify({ requestSecret: `REQUEST_SECRET_${id}`, text: 'PRIVATE_TEXT' }),
    } });
    chrome.debugger.onEvent.fire({ tabId: 17 }, 'Network.responseReceived', { requestId: id, response: {
      status: 200, headers: { 'Content-Type': 'application/json', Cookie: 'COOKIE_SECRET' },
    } });
    chrome.debugger.onEvent.fire({ tabId: 17 }, 'Network.loadingFinished', { requestId: id });
  }

  assert.equal((await send({ type: 'start', duration: 1, tabId: 999 })).error, 'The active tab must be messages.google.com over HTTPS.');
  assert.equal(calls.some(call => call[0] === 'attach'), false);
  assert.equal(await send({ type: 'start', duration: 1 }, { id: 'foreign-extension' }), undefined);
  assert.equal(await send({ type: 'start', duration: 1 }, { id: 'observer-test', tab: { id: 17 } }), undefined);
  chrome.tabs.active = { id: 17, url: 'https://messages.google.com/' };
  assert.match((await send({ type: 'start', duration: 20 })).message, /Observing/);
  assert.deepEqual(calls.filter(call => call[0] === 'Network.enable')[0][2], {
    maxTotalBufferSize: 2 * 1024 * 1024, maxResourceBufferSize: 1024 * 1024, maxPostDataSize: 1024 * 1024,
  });
  rpcEvents('old-request');
  await waitFor(() => calls.some(call => call[0] === 'Network.getResponseBody'));
  await send({ type: 'stop' });
  await send({ type: 'start', duration: 20 });
  bodyWait.resolve(bodyPayload(JSON.stringify({ staleSecret: 'STALE_RESPONSE_SECRET' })));
  await new Promise(resolve => setTimeout(resolve, 10));
  assert.deepEqual((await send({ type: 'snapshot' })).records, []);

  rpcEvents('new-request');
  await waitFor(async () => calls.filter(call => call[0] === 'Network.getResponseBody').length >= 2);
  await new Promise(resolve => setTimeout(resolve, 5));
  const snapshot = await send({ type: 'snapshot' });
  assert.equal(snapshot.records.length, 3);
  assert.deepEqual(snapshot.records.map(record => record.type), ['request', 'response', 'response']);
  assert.equal(snapshot.records[0].body.propertyCount, 2);
  assert.equal(snapshot.records[1].status, 200);
  const serialized = JSON.stringify(snapshot.records);
  for (const secret of ['URL_SECRET', 'HEADER_SECRET', 'COOKIE_SECRET', 'requestSecret', 'REQUEST_SECRET', 'PRIVATE_TEXT', 'STALE_RESPONSE_SECRET', 'NEW_RESPONSE_SECRET']) {
    assert.equal(serialized.includes(secret), false, `record leaked ${secret}`);
  }
  await send({ type: 'stop' });
  const commandNames = debuggerCommands;
  assert.ok(commandNames.length > 0);
  assert.equal(commandNames.every(name => name === 'Network.enable' || name === 'Network.getResponseBody'), true);
  assert.ok(calls.filter(call => call[0] === 'detach').length >= 2);
  assert.equal(calls.filter(call => call[0] === 'attach').every(call => call[1].tabId === 17), true);
  assert.equal(calls.some(call => ['Page.navigate', 'Runtime.evaluate', 'Target.createTarget', 'Network.setRequestInterception'].includes(call[0])), false);
  await send({ type: 'start', duration: 20 });
  chrome.tabs.onUpdated.fire(17, { url: 'https://example.test/' });
  await waitFor(async () => !(await send({ type: 'snapshot' })).active);
  await waitFor(() => calls.filter(call => call[0] === 'detach').length >= 3);
});
