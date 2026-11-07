// A walk through the main features against the real Rust side, clicking the
// page like a person would. Loaded only by a debug build started with
// TACKNOTE_SELFTEST, which should also set TACKNOTE_TEST_DIR to an empty folder.

import { t } from './strings.js';

export async function run({ invoke }) {
  const results = [];
  const check = (ok, name) => results.push(`${ok ? 'PASS' : 'FAIL'} ${name}`);
  const $ = (selector) => document.querySelector(selector);
  const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
  async function until(condition, ms = 3000) {
    for (const end = Date.now() + ms; Date.now() < end; await wait(50)) {
      if (await condition()) return true;
    }
    return false;
  }
  const shown = (id) => !document.getElementById(id).hidden;
  const box = () => $('#note-text');
  const cards = () => [...document.querySelectorAll('.card')];
  const saved = async (id) => (await invoke('get_note', { id }))?.text;
  function fill(input, value) {
    input.value = value;
    input.setSelectionRange?.(value.length, value.length);
    input.dispatchEvent(new Event('input', { bubbles: true }));
  }

  try {
    check(await until(() => shown('empty') && cards().length === 0), 'a fresh start shows the empty list');

    $('#new-note').click();
    check(await until(() => shown('editor')), 'the new-note button opens the editor');
    const id = decodeURIComponent(location.hash.split('/')[2]);
    fill(box(), '买牛奶\n[ ] 鸡蛋');
    check(await until(async () => (await saved(id)) === '买牛奶\n[ ] 鸡蛋'), 'typing is saved');

    // Rule 5: a change brought by a sync shows in the open editor.
    box().focus();
    box().setSelectionRange(1, 1);
    await invoke('selftest_remote_edit', { id, text: '买牛奶和面包\n[ ] 鸡蛋' });
    check(await until(() => box().value === '买牛奶和面包\n[ ] 鸡蛋'), 'the open editor shows a change from a sync');
    check(box().selectionStart === 1, 'the caret stays where it was');
    fill(box(), `${box().value}\n葡萄`);
    $('#add-checkbox').click();
    check(
      await until(async () => (await saved(id)) === '买牛奶和面包\n[ ] 鸡蛋\n[ ] 葡萄'),
      'typing after a sync keeps the synced text, and the checkbox button adds a to-do',
    );

    $('#toggle-pin').click();
    $('#colors [data-color="2"]').click();
    check(
      await until(async () => {
        const note = await invoke('get_note', { id });
        return note.pinned && note.colorIndex === 2;
      }),
      'pin and colour are saved',
    );
    check(await until(() => $('#editor').classList.contains('color-2')), 'the editor takes the new colour');

    $('#editor-back').click();
    check(await until(() => shown('home') && cards().length === 1), 'back to the list with one card');
    const card = () => $(`.card[data-id="${id}"]`);
    check(card()?.classList.contains('color-2') && !!card().querySelector('.pin-mark'), 'the card shows its colour and pin');

    card().querySelector('.check-line').click();
    check(await until(async () => (await saved(id))?.includes('[x] 鸡蛋')), 'ticking a to-do on the card saves it');
    check(await until(() => card()?.querySelector('.check-line.checked')), 'the card shows the tick');

    $('#new-note').click();
    await until(() => shown('editor'));
    fill(box(), 'Hello world');
    $('#editor-back').click();
    check(await until(() => shown('home') && cards().length === 2), 'a second note appears in the list');
    check(cards()[0]?.dataset.id === id, 'the pinned note comes first');

    const search = $('#search');
    fill(search, '牛奶');
    check(cards().length === 1 && cards()[0].dataset.id === id, 'search finds the matching note');
    fill(search, 'hello');
    check(cards().length === 1 && cards()[0].dataset.id !== id, 'search ignores case');
    fill(search, '不存在');
    check(cards().length === 0 && shown('empty'), 'a search without matches says so');
    fill(search, '');
    check(cards().length === 2, 'clearing the search shows every note');

    $('#new-note').click();
    await until(() => shown('editor'));
    $('#editor-back').click();
    check(
      await until(async () => shown('home') && (await invoke('list_notes')).length === 2),
      'a new note left empty is discarded',
    );

    cards().find((el) => el.dataset.id !== id).click();
    await until(() => shown('editor'));
    $('#delete-note').click();
    check(
      await until(async () => shown('home') && cards().length === 1 && (await invoke('list_notes')).length === 1),
      'deleting a note removes it',
    );

    // Sync settings and the system keychain.
    $('#sync-button').click();
    check(await until(() => shown('settings') && !$('#webdav-url').disabled, 15000), 'the sync button opens the settings');
    fill($('#webdav-url'), 'http://example.com/dav/');
    check(await until(() => !$('#insecure-warning').hidden), 'plain http to another computer gets a warning');
    fill($('#webdav-url'), 'http://127.0.0.1:9/');
    check(await until(() => $('#insecure-warning').hidden), 'plain http to this computer does not');
    const password = `selftest-${Math.random().toString(36).slice(2)}`;
    fill($('#webdav-username'), 'selftest');
    fill($('#webdav-password'), password);
    $('#save-settings').click();
    const unreachable = t.status('error', 'network');
    check(
      await until(() => $('#settings-status').textContent.startsWith(unreachable), 15000),
      'saving starts a sync, which reports the unreachable server',
    );
    check($('#settings-error').hidden, 'saving reports no error');
    const settings = await invoke('get_settings');
    check(settings.password === password && !settings.passwordUnavailable, 'the keychain gives the password back');
  } catch (error) {
    check(false, `stopped early: ${error?.detail ?? error}`);
  }
  const passed = results.every((line) => line.startsWith('PASS'));
  await invoke('selftest_finish', { report: results.join('\n'), passed });
}
