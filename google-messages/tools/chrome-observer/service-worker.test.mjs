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

test('worker validates the active tab, uses only Network CDP calls, and isolates stale body replies', async t => {
  const calls = [];
  const debuggerCommands = [];
  const bodyWait = deferred();
  const nativeWait = deferred();
  const nativeCalls = [];
  const bodyPayload = text => ({ body: btoa(text), base64Encoded: true });
  const chrome = {
    runtime: {
      id: 'observer-test', onMessage: eventHook(), sendMessage: async () => {},
      sendNativeMessage: async (host, payload) => {
        nativeCalls.push({ host, payload: structuredClone(payload) });
        if (nativeCalls.length === 1) return nativeWait.promise;
        if (payload.type === 'gaia_lookup_inspect') return { ok: false, error: 'http_error', http_status: 400, rpc_status: 'INVALID_ARGUMENT', local_description: 'PRIVATE_LOCAL_DESCRIPTION', metadata: 'PRIVATE_METADATA' };
        return { ok: true, sources: 3, token: 'native-secret' };
      },
    },
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
  assert.deepEqual(manifest.permissions.sort(), ['activeTab', 'alarms', 'debugger', 'nativeMessaging']);
  for (const forbidden of ['host_permissions', 'tabs', 'scripting', 'cookies', 'storage', 'downloads']) {
    assert.equal(Object.hasOwn(manifest, forbidden), false);
  }
  const worker = await import(`./service-worker.js?test=${Date.now()}`);

  async function send(payload, sender = { id: 'observer-test' }) {
    return new Promise(resolve => {
      const asyncReply = chrome.runtime.onMessage.listener()(payload, sender, resolve);
      if (asyncReply !== true) resolve(undefined);
    });
  }
  t.after(async () => { await send({ type: 'stop' }); });
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

  const previousRecordCount = (await send({ type: 'snapshot' })).records.length;
  const bodyCallsBeforeAuth = debuggerCommands.filter(name => name === 'Network.getResponseBody').length;
  chrome.tabs.active = { id: 17, url: 'https://messages.google.com/' };
  assert.match((await send({ type: 'auth-probe' })).message, /waiting/);
  assert.equal(calls.filter(call => call[0] === 'Network.enable').at(-1)[2].maxPostDataSize, 0);
  const authPath = '/$rpc/google.internal.communications.instantmessaging.v1.Registration/SignInGaia';
  chrome.debugger.onEvent.fire({ tabId: 17 }, 'Network.requestWillBeSent', { requestId: 'wrong-host', request: {
    url: `https://sub.instantmessaging-pa.googleapis.com${authPath}`, method: 'POST', headers: { Authorization: 'WRONG_HOST_SECRET' },
  } });
  const firstProbeId = 'matching-auth-request';
  chrome.debugger.onEvent.fire({ tabId: 17 }, 'Network.requestWillBeSentExtraInfo', { requestId: firstProbeId, headers: {
    authorization: 'Bearer EXTRA_AUTH_SECRET', 'x-goog-api-key': 'EXTRA_API_SECRET',
    'x-goog-authuser': '5', origin: 'https://messages.google.com', cookie: 'COOKIE_SECRET',
  } });
  chrome.debugger.onEvent.fire({ tabId: 17 }, 'Network.requestWillBeSent', { requestId: firstProbeId, hasExtraInfo: true, request: {
    url: `https://instantmessaging-pa.googleapis.com${authPath}?query=AUTH_URL_SECRET`, method: 'POST',
    headers: { Authorization: 'Bearer BASE_AUTH_SECRET', 'X-Goog-Api-Key': 'BASE_API_SECRET', Cookie: 'COOKIE_SECRET' },
    postData: 'AUTH_BODY_SECRET',
  } });
  chrome.debugger.onEvent.fire({ tabId: 17 }, 'Network.responseReceived', { requestId: firstProbeId, response: { status: 200, headers: { Cookie: 'RESPONSE_COOKIE_SECRET' } } });
  chrome.debugger.onEvent.fire({ tabId: 17 }, 'Network.loadingFinished', { requestId: firstProbeId });
  await waitFor(() => nativeCalls.length === 1);
  assert.equal(nativeCalls[0].host, 'com.handover.google_messages.auth_probe');
  assert.deepEqual(Object.keys(nativeCalls[0].payload).sort(), ['api_key', 'auth_user', 'authorization', 'endpoint', 'origin', 'type']);
  assert.deepEqual(nativeCalls[0].payload, {
    type: 'gaia_lookup', endpoint: 'https://instantmessaging-pa.googleapis.com',
    origin: 'https://messages.google.com', authorization: 'Bearer EXTRA_AUTH_SECRET',
    api_key: 'EXTRA_API_SECRET', auth_user: '5',
  });
  assert.equal((await send({ type: 'snapshot' })).records.length, previousRecordCount);
  assert.equal((await send({ type: 'snapshot' })).nativeProbe.state, 'running');

  await send({ type: 'auth-probe' });
  nativeWait.resolve({ ok: true, sources: 999, http_status: 999, error: 'NATIVE_ERROR_SECRET', authorization: 'NATIVE_AUTH_SECRET' });
  await new Promise(resolve => setTimeout(resolve, 5));
  assert.equal((await send({ type: 'snapshot' })).nativeProbe.state, 'waiting', 'old native reply must not update a new probe');
  const secondId = 'second-auth-request';
  chrome.debugger.onEvent.fire({ tabId: 17 }, 'Network.requestWillBeSent', { requestId: secondId, hasExtraInfo: false, request: {
    url: `https://instantmessaging-pa-jms-us.clients6.google.com${authPath}`, method: 'POST',
    headers: { Authorization: 'Bearer SECOND_AUTH_SECRET', 'X-Goog-Api-Key': 'SECOND_API_SECRET' },
  } });
  chrome.debugger.onEvent.fire({ tabId: 17 }, 'Network.responseReceived', { requestId: secondId, hasExtraInfo: false, response: { status: 204 } });
  await waitFor(() => nativeCalls.length === 2);
  await waitFor(async () => (await send({ type: 'snapshot' })).nativeProbe.state === 'complete');
  const finalSnapshot = await send({ type: 'snapshot' });
  assert.deepEqual(finalSnapshot.nativeProbe, { state: 'complete', result: { ok: true, sources: 3 } });
  const safeState = JSON.stringify({ records: finalSnapshot.records, nativeProbe: finalSnapshot.nativeProbe });
  for (const secret of ['EXTRA_AUTH_SECRET', 'EXTRA_API_SECRET', 'COOKIE_SECRET', 'RESPONSE_COOKIE_SECRET', 'AUTH_URL_SECRET', 'AUTH_BODY_SECRET', 'WRONG_HOST_SECRET', 'NATIVE_ERROR_SECRET', 'NATIVE_AUTH_SECRET', 'SECOND_AUTH_SECRET', 'SECOND_API_SECRET', 'native-secret']) {
    assert.equal(safeState.includes(secret), false, `snapshot leaked ${secret}`);
  }
  assert.equal(debuggerCommands.filter(name => name === 'Network.getResponseBody').length, bodyCallsBeforeAuth, 'auth mode must not fetch response bodies');

  await send({ type: 'auth-probe' });
  chrome.debugger.onEvent.fire({ tabId: 17 }, 'Network.requestWillBeSentExtraInfo', { requestId: 'stopped-secret', headers: { Authorization: 'STOP_SECRET' } });
  await send({ type: 'stop' });
  assert.deepEqual((await send({ type: 'snapshot' })).nativeProbe, { state: 'failed', result: { error: 'stopped' } });
  // Cookies are discarded while ExtraInfo has no matched request URL.
  await send({ type: 'auth-probe-with-cookies' });
  chrome.debugger.onEvent.fire({ tabId: 17 }, 'Network.requestWillBeSentExtraInfo', {
    requestId: 'reverse-cookie', headers: {
      Authorization: 'Bearer REVERSE_AUTH', 'X-Goog-Api-Key': 'REVERSE_KEY', Cookie: 'SID=REVERSE_COOKIE_SECRET',
    },
  });
  chrome.debugger.onEvent.fire({ tabId: 17 }, 'Network.requestWillBeSent', {
    requestId: 'reverse-cookie', request: { url: `https://instantmessaging-pa.clients6.google.com${authPath}`, method: 'POST', headers: {} },
  });
  await waitFor(async () => (await send({ type: 'snapshot' })).nativeProbe.state === 'failed');
  assert.equal((await send({ type: 'snapshot' })).nativeProbe.result.error, 'cookie_unavailable');
  assert.equal(nativeCalls.length, 2);

  await send({ type: 'auth-probe-with-cookies' });
  chrome.debugger.onEvent.fire({ tabId: 17 }, 'Network.requestWillBeSentExtraInfo', {
    requestId: 'unmatched-cookie', headers: { Cookie: 'SID=UNRELATED_COOKIE_SECRET' },
  });
  chrome.debugger.onEvent.fire({ tabId: 17 }, 'Network.requestWillBeSent', {
    requestId: 'cookie-target', request: {
      url: `https://instantmessaging-pa.clients6.google.com${authPath}`, method: 'POST', headers: {},
    },
  });
  chrome.debugger.onEvent.fire({ tabId: 17 }, 'Network.requestWillBeSentExtraInfo', {
    requestId: 'cookie-target', headers: {
      Authorization: 'Bearer COOKIE_MODE_AUTH', 'X-Goog-Api-Key': 'COOKIE_MODE_KEY',
      Cookie: 'SID=APPROVED_SERVICE_COOKIE_SECRET', Origin: 'https://messages.google.com',
    },
  });
  await waitFor(() => nativeCalls.length === 3);
  assert.equal(nativeCalls[2].payload.type, 'gaia_lookup_with_cookies');
  assert.equal(nativeCalls[2].payload.service_cookie, 'SID=APPROVED_SERVICE_COOKIE_SECRET');
  assert.equal(nativeCalls[2].payload.endpoint, 'https://instantmessaging-pa.clients6.google.com');
  await waitFor(async () => (await send({ type: 'snapshot' })).nativeProbe.state === 'complete');
  const cookieSnapshot = JSON.stringify(await send({ type: 'snapshot' }));
  for (const secret of ['APPROVED_SERVICE_COOKIE_SECRET', 'UNRELATED_COOKIE_SECRET', 'REVERSE_COOKIE_SECRET', 'COOKIE_MODE_AUTH', 'COOKIE_MODE_KEY']) {
    assert.equal(cookieSnapshot.includes(secret), false);
  }
  const lastEnable = calls.filter(call => call[0] === 'Network.enable').at(-1);
  assert.equal(lastEnable[2].maxPostDataSize, 0);
  await send({ type: 'auth-probe-browser-request' });
  const browserBody = [['12345678-1234-4234-8234-123456789abc', null, 'GDitto', null, null, null, [null, null, 20261001, 2, 0, null, 4, null, 6]], [[3, 'messages-web-0123456789abcdef0123456789abcdef']], 1, 'GDitto'];
  const headers = { Authorization: 'Bearer BROWSER_AUTH', 'X-Goog-Api-Key': 'BROWSER_KEY', Cookie: 'SID=BROWSER_COOKIE' };
  for (const [id, body] of [['registration', browserBody.with(2, 0)], ['lookup', browserBody]]) {
    chrome.debugger.onEvent.fire({ tabId: 17 }, 'Network.requestWillBeSent', { requestId: id, request: {
      url: `https://instantmessaging-pa.clients6.google.com${authPath}`, method: 'POST', headers, postData: JSON.stringify(body),
    } });
    chrome.debugger.onEvent.fire({ tabId: 17 }, 'Network.responseReceived', { requestId: id, hasExtraInfo: false });
    if (id === 'registration') assert.equal(nativeCalls.length, 3);
  }
  await waitFor(() => nativeCalls.length === 4);
  assert.equal(nativeCalls[3].payload.type, 'gaia_lookup_browser_request');
  assert.deepEqual(nativeCalls[3].payload.browser_request, browserBody);
  assert.equal(nativeCalls[3].payload.service_cookie, 'SID=BROWSER_COOKIE');
  await waitFor(async () => (await send({ type: 'snapshot' })).nativeProbe.state === 'complete');
  const browserSnapshot = JSON.stringify(await send({ type: 'snapshot' }));
  for (const privateValue of ['12345678-1234-4234-8234-123456789abc', 'messages-web-0123456789abcdef0123456789abcdef', 'BROWSER_AUTH', 'BROWSER_COOKIE']) assert.equal(browserSnapshot.includes(privateValue), false);
  await send({ type: 'auth-probe-inspect' });
  chrome.debugger.onEvent.fire({ tabId: 17 }, 'Network.requestWillBeSent', { requestId: 'inspect', request: {
    url: `https://instantmessaging-pa.clients6.google.com${authPath}`, method: 'POST', headers, postData: JSON.stringify(browserBody),
  } });
  chrome.debugger.onEvent.fire({ tabId: 17 }, 'Network.responseReceived', { requestId: 'inspect', hasExtraInfo: false });
  await waitFor(() => nativeCalls.length === 5);
  await waitFor(async () => (await send({ type: 'snapshot' })).nativeProbe.state === 'failed');
  const inspectSnapshot = JSON.stringify(await send({ type: 'snapshot' }));
  assert.equal(inspectSnapshot.includes('PRIVATE_LOCAL_DESCRIPTION'), false);
  assert.equal(inspectSnapshot.includes('PRIVATE_METADATA'), false);
  assert.deepEqual(await send({ type: 'take-local-description' }), { description: 'PRIVATE_LOCAL_DESCRIPTION' });
  assert.deepEqual(await send({ type: 'take-local-description' }), { description: null });
  await send({ type: 'auth-probe-inspect' });
  const originalTimeout = globalThis.setTimeout;
  let expireDescription;
  globalThis.setTimeout = (callback, delay, ...args) => {
    if (delay === 60_000) expireDescription = callback;
    return originalTimeout(callback, delay, ...args);
  };
  try {
    chrome.debugger.onEvent.fire({ tabId: 17 }, 'Network.requestWillBeSent', { requestId: 'inspect-expiry', request: {
      url: `https://instantmessaging-pa.clients6.google.com${authPath}`, method: 'POST', headers, postData: JSON.stringify(browserBody),
    } });
    chrome.debugger.onEvent.fire({ tabId: 17 }, 'Network.responseReceived', { requestId: 'inspect-expiry', hasExtraInfo: false });
    await waitFor(() => typeof expireDescription === 'function');
    expireDescription();
    assert.deepEqual(await send({ type: 'take-local-description' }), { description: null });
  } finally { globalThis.setTimeout = originalTimeout; }
  assert.equal(debuggerCommands.filter(name => name === 'Network.getResponseBody').length, bodyCallsBeforeAuth);
  await assert.rejects(worker.withTimeout(new Promise(() => {}), 5, 'native_timeout'), /native_timeout/);
  assert.deepEqual(await worker.nativeReplyWithTimeout(new Promise(() => {}), 5), { error: 'timeout' });
  assert.deepEqual(worker.sanitizeNativeReply({ ok: false, error: 'http_error', http_status: 400, rpc_status: 'INVALID_ARGUMENT', message: 'PRIVATE_ERROR_MESSAGE' }), {
    state: 'failed', result: { ok: false, error: 'http_error', http_status: 400, rpc_status: 'INVALID_ARGUMENT' },
  });
  assert.deepEqual(worker.sanitizeNativeReply({ ok: false, error: 'http_error', http_status: 400, rpc_status: 'INVALID_ARGUMENT', rpc_reason: 'API_KEY_INVALID', metadata: { key: 'PRIVATE_KEY' } }), {
    state: 'failed', result: { ok: false, error: 'http_error', http_status: 400, rpc_status: 'INVALID_ARGUMENT', rpc_reason: 'API_KEY_INVALID' },
  });
  for (const reply of [
    { ok: false, error: 'http_error', rpc_reason: 'API_KEY_INVALID' },
    { ok: false, error: 'http_error', rpc_status: 'INVALID_ARGUMENT', rpc_reason: 'PRIVATE_ERROR_MESSAGE' },
    { ok: true, sources: 4, rpc_reason: 'API_KEY_INVALID' },
    { ok: false, error: 'http_error', http_status: 400, rpc_status: 'PRIVATE_ERROR_MESSAGE' },
    { ok: false, error: 'network', rpc_status: 'INVALID_ARGUMENT' },
    { ok: true, sources: 4, rpc_status: 'INVALID_ARGUMENT' },
  ]) assert.deepEqual(worker.sanitizeNativeReply(reply), { state: 'failed', result: { error: 'invalid_response' } });
  assert.deepEqual(worker.sanitizeNativeReply({ ok: true, sources: 4, http_status: 204 }), { state: 'failed', result: { error: 'invalid_response' } });
  assert.deepEqual(worker.sanitizeNativeReply({ ok: false, error: 'http_error', http_status: 401, message: 'secret' }), {
    state: 'failed', result: { ok: false, error: 'http_error', http_status: 401 },
  });
});
