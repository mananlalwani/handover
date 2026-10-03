const status = document.querySelector('#status');
const nativeStatus = document.querySelector('#native-status');
const duration = document.querySelector('#duration');
const download = document.querySelector('#download');
let exportUrl = null;

function clearExport() {
  if (exportUrl) URL.revokeObjectURL(exportUrl);
  exportUrl = null;
  download.removeAttribute('href');
  download.hidden = true;
}

async function message(payload) {
  try { return await chrome.runtime.sendMessage(payload); }
  catch { return { error: 'Observer is unavailable.' }; }
}

async function refresh() {
  const state = await message({ type: 'snapshot' });
  if (!state || state.error) { status.textContent = state?.error ?? 'Observer is unavailable.'; return; }
  status.textContent = state.active
    ? `Observing this tab. ${state.count} sanitized records so far.`
    : `Stopped. ${state.count} sanitized records available.`;
  const probe = state.nativeProbe;
  if (probe?.state === 'waiting' || probe?.state === 'running') clearLocalView();
  if (probe?.state === 'complete') {
    const result = probe.result ?? {};
    nativeStatus.textContent = result.ok && result.registered
      ? 'Native device registered. The unpaired credential was saved locally.'
      : result.ok
      ? `Native authentication check complete. ${Number.isInteger(result.sources) ? `${result.sources} source(s) found.` : ''}`
      : 'Native authentication check complete.';
  } else if (probe?.state === 'failed') {
    const result = probe.result ?? {};
    if (result.registration) {
      nativeStatus.textContent = result.error === 'sign_in_not_observed'
        ? 'No matching sign-in request was seen; no registration request was sent.'
        : 'Registration outcome was not confirmed. Check Google Messages registered devices before trying again.';
    } else {
      const statusCode = result.error === 'http_error' && Number.isInteger(result.http_status) ? `, HTTP ${result.http_status}` : '';
      const rpcStatus = typeof result.rpc_status === 'string' ? `, RPC ${result.rpc_status}` : '';
      const rpcReason = typeof result.rpc_reason === 'string' ? `, reason ${result.rpc_reason}` : '';
      nativeStatus.textContent = `Native authentication probe failed (${result.error ?? 'invalid_response'}${statusCode}${rpcStatus}${rpcReason}).`;
    }
  } else nativeStatus.textContent = `Native authentication probe: ${probe?.state ?? 'idle'}.`;
}

document.querySelector('#start').addEventListener('click', async () => {
  const seconds = Number(duration.value);
  if (!Number.isInteger(seconds) || seconds < 1 || seconds > 300) { status.textContent = 'Choose a duration from 1 to 300 seconds.'; return; }
  const [tab] = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
  if (!tab || !Number.isInteger(tab.id) || !tab.url) { status.textContent = 'Could not inspect the active tab.'; return; }
  const result = await message({ type: 'start', duration: seconds });
  if (result?.error) { status.textContent = result.error; return; }
  status.textContent = result?.error ?? 'Observer started.';
  clearExport();
  await refresh();
});

document.querySelector('#auth-probe').addEventListener('click', async () => {
  const [tab] = await chrome.tabs.query({ active: true, lastFocusedWindow: true });
  if (!tab || !Number.isInteger(tab.id) || !tab.url) { nativeStatus.textContent = 'Could not inspect the active tab.'; return; }
  const result = await message({ type: 'auth-probe' });
  nativeStatus.textContent = result?.error ?? 'Probe started. Refresh the current tab to trigger SignInGaia.';
  await refresh();
});

document.querySelector('#stop').addEventListener('click', async () => {
  const result = await message({ type: 'stop' });
  status.textContent = result?.error ?? 'Observer stopped.';
  await refresh();
});

document.querySelector('#cookie-probe').addEventListener('click', async () => {
  const result = await message({ type: 'auth-probe-with-cookies' });
  nativeStatus.textContent = result?.error ?? 'Cookie comparison started. Refresh Google Messages, then reopen this popup.';
  await refresh();
});

document.querySelector('#browser-request-probe').addEventListener('click', async () => {
  const result = await message({ type: 'auth-probe-browser-request' });
  nativeStatus.textContent = result?.error ?? 'Browser lookup comparison started. Refresh Google Messages, then reopen this popup.';
  await refresh();
});

document.querySelector('#register-device').addEventListener('click', async () => {
  const result = await message({ type: 'register-device' });
  nativeStatus.textContent = result?.error ?? 'Registration armed. Refresh Google Messages to submit one native registration request.';
  await refresh();
});

document.querySelector('#save').addEventListener('click', async () => {
  const state = await message({ type: 'snapshot' });
  if (!state || state.error) { status.textContent = state?.error ?? 'Observer is unavailable.'; return; }
  clearExport();
  const blob = new Blob([JSON.stringify(state.records, null, 2), '\n'], { type: 'application/json' });
  exportUrl = URL.createObjectURL(blob);
  download.href = exportUrl;
  download.download = 'google-messages-rpc-observation.json';
  download.hidden = false;
  download.textContent = `Save ${state.records.length} sanitized records`;
  status.textContent = 'Export is ready. Use the link to save it.';
});

window.addEventListener('unload', clearExport);

chrome.runtime.onMessage.addListener(message => {
  if (message?.type === 'state-changed') refresh();
});
refresh();

const localView = document.querySelector('#local-description');
let localViewTimer = null;
function clearLocalView() {
  localView.textContent = '';
  localView.hidden = true;
  if (localViewTimer !== null) clearTimeout(localViewTimer);
  localViewTimer = null;
}

document.querySelector('#inspect-probe').addEventListener('click', async () => {
  clearLocalView();
  const result = await message({ type: 'auth-probe-inspect' });
  nativeStatus.textContent = result?.error ?? 'Local inspection started. Refresh Messages, then reopen the popup and view the description once.';
  await refresh();
});

document.querySelector('#view-local-description').addEventListener('click', async () => {
  clearLocalView();
  const result = await message({ type: 'take-local-description' });
  localView.textContent = typeof result?.description === 'string' && new TextEncoder().encode(result.description).length <= 2048
    ? result.description : 'No local error description is available. It may have expired or the server omitted it.';
  localView.hidden = false;
  localViewTimer = setTimeout(clearLocalView, 60_000);
});
window.addEventListener('unload', clearLocalView);
