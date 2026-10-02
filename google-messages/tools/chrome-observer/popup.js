const status = document.querySelector('#status');
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

document.querySelector('#stop').addEventListener('click', async () => {
  const result = await message({ type: 'stop' });
  status.textContent = result?.error ?? 'Observer stopped.';
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
