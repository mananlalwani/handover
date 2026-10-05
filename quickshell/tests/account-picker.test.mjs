import assert from 'node:assert/strict';
import fs from 'node:fs';
import vm from 'node:vm';
import test from 'node:test';
const picker = vm.createContext({});
vm.runInContext(fs.readFileSync(new URL('../AccountPicker.js', import.meta.url), 'utf8').replace('.pragma library', ''), picker);
const offline = { id: 'one', label: 'Google Messages', connected: false, authenticated: false };
const online = { id: 'two', label: 'Google Messages', connected: true, authenticated: true };
test('duplicate names identify distinct instances and current connection state', () => {
    const choices = picker.choices([online, offline]);
    assert.equal(new Set(choices.map(account => account.displayLabel)).size, 2);
    assert.equal(choices[0].displayLabel, 'Google Messages 1 · Offline');
    assert.equal(choices[1].displayLabel, 'Google Messages 2 · Connected');
    assert.equal(offline.displayLabel, undefined);
});
test('updates and reordering preserve the chosen account identity', () => {
    const before = picker.choices([offline, online]);
    const after = picker.choices([{ ...online, connected: false }, { ...offline, connected: true, authenticated: true }]);
    assert.equal(picker.selectedId(after, 'two'), 'two');
    assert.equal(before[1].id, after[1].id);
    assert.equal(after[1].displayLabel, 'Google Messages 2 · Offline');
});
test('missing selections prefer a connected account and handle removal of all accounts', () => {
    assert.equal(picker.selectedId(picker.choices([offline, online]), ''), 'two');
    assert.equal(picker.selectedId([offline], 'two'), 'one');
    assert.equal(picker.selectedId([], 'two'), '');
});
