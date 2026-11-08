// A walk through the main features against the real Rust side, clicking the
// page like a person would. Loaded only by a debug build started with
// TACKNOTE_SELFTEST, which should also set TACKNOTE_TEST_DIR to an empty folder.

import { t } from './strings.js';

const { emit, listen } = window.__TAURI__.event;
const wait = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

async function until(condition, ms = 3000) {
  for (const end = Date.now() + ms; Date.now() < end; await wait(50)) {
    if (await condition()) return true;
  }
  return false;
}

/** Has a sticky window do something, then answers with what it shows. */
async function askSticky(noteId, action, extra = {}) {
  const request = Math.random().toString(36).slice(2);
  const send = () => emit('selftest-sticky', { id: noteId, action, request, ...extra });
  if (action === 'close') return send();
  let answer = null;
  const unlisten = await listen('selftest-sticky-reply', ({ payload }) => {
    if (payload.request === request) answer = payload;
  });
  try {
    let sent = Date.now();
    await send();
    for (const end = sent + 10000; !answer && Date.now() < end; await wait(50)) {
      // Only a question is asked again, for a window still loading.
      if (action === 'state' && Date.now() - sent > 500) {
        sent = Date.now();
        await send();
      }
    }
  } finally {
    unlisten();
  }
  if (!answer) throw new Error(`the sticky window did not answer "${action}"`);
  return answer;
}

/** The sticky window's side of `askSticky`. */
export function serveSticky({ id, box, onTopButton, closeButton }) {
  listen('selftest-sticky', async ({ payload }) => {
    if (payload.id !== id) return;
    if (payload.action === 'type') {
      box.value = payload.text;
      box.dispatchEvent(new Event('input', { bubbles: true }));
    } else if (payload.action === 'toggleOnTop') {
      onTopButton.click();
    } else if (payload.action === 'close') {
      closeButton.click();
      return;
    }
    await wait(100);
    emit('selftest-sticky-reply', {
      request: payload.request,
      value: box.value,
      colorClass: [...document.body.classList].find((name) => name.startsWith('color-')),
      theme: document.documentElement.dataset.theme,
      onTop: onTopButton.getAttribute('aria-pressed') === 'true',
    });
  });
}

export async function run({ invoke, mode }) {
  const results = [];
  const check = (ok, name) => results.push(`${ok ? 'PASS' : 'FAIL'} ${name}`);
  const finish = () => invoke('selftest_finish', {
    report: results.join('\n'),
    passed: results.length > 0 && results.every((line) => line.startsWith('PASS')),
  });
  if (mode === 'restore') {
    try {
      let info;
      await until(async () => (info = (await invoke('selftest_windows')).find((w) => w.label.startsWith('sticky-'))), 10000);
      check(info && Math.round(info.x) === 400 && Math.round(info.y) === 300, 'a sticky window comes back in its place after a restart');
      check(info?.onTop === true, 'and still stays on top');
      check((await askSticky(info.label.slice('sticky-'.length), 'state')).value === '贴在桌面上', 'and shows its text');
    } catch (error) {
      check(false, `stopped early: ${error?.detail ?? error}`);
    }
    await finish();
    return;
  }
  const themeBefore = localStorage.getItem('tacknote.theme');
  const $ = (selector) => document.querySelector(selector);
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
    // An empty note: the editor sits in its place and nothing covers the colours.
    check(getComputedStyle(box()).position !== 'absolute' && box().getBoundingClientRect().top < 120, 'an empty note shows its editor at the top');
    const swatch = $('#colors [data-color="1"]').getBoundingClientRect();
    check(
      document.elementFromPoint(swatch.left + swatch.width / 2, swatch.top + swatch.height / 2) === $('#colors [data-color="1"]'),
      'the colours can be tapped in an empty note',
    );
    const id = decodeURIComponent(location.hash.split('/')[2]);
    fill(box(), '买牛奶\n[ ] 鸡蛋');
    check(await until(async () => (await saved(id)) === '买牛奶\n[ ] 鸡蛋'), 'typing is saved');
    check(!box().innerText.includes('[ ]') && Boolean(box().querySelector('.ed-box')), 'the editor shows a to-do as a box, not brackets');
    box().querySelector('.ed-box').click();
    check(await until(async () => (await saved(id)) === '买牛奶\n[x] 鸡蛋'), 'tapping the box in the editor ticks it');
    box().querySelector('.ed-box').click();
    check(await until(async () => (await saved(id)) === '买牛奶\n[ ] 鸡蛋'), 'and tapping it again unticks it');
    box().focus();
    box().setSelectionRange(box().value.length);
    const key = (inputType) => box().dispatchEvent(new InputEvent('beforeinput', { inputType, bubbles: true, cancelable: true }));
    key('insertParagraph');
    check(box().value === '买牛奶\n[ ] 鸡蛋\n[ ] ', 'Enter on a to-do starts another');
    key('insertParagraph');
    check(box().value === '买牛奶\n[ ] 鸡蛋\n', 'Enter on an empty to-do ends the list');
    key('deleteContentBackward');
    check(await until(async () => (await saved(id)) === '买牛奶\n[ ] 鸡蛋'), 'Backspace at a line start joins it to the line above');

    // Rule 5: a change brought by a sync shows in the open editor.
    box().focus();
    box().setSelectionRange(1, 1);
    await invoke('selftest_remote_edit', { id, text: '买牛奶和面包\n[ ] 鸡蛋' });
    check(await until(() => box().value === '买牛奶和面包\n[ ] 鸡蛋'), 'the open editor shows a change from a sync');
    check(box().selectionStart === 1, 'the caret stays where it was');
    fill(box(), `${box().value}\n葡萄`);
    $('#add-checkbox').click();
    $('#add-checkbox').click();
    check(box().value.endsWith('\n葡萄'), 'pressing the to-do button twice gives back a plain line');
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

    card().querySelector('.check-line span').click();
    check(await until(() => shown('editor')), 'tapping the text of a to-do opens the note');
    check(!(await saved(id)).includes('[x] 鸡蛋'), 'and does not tick it');
    $('#editor-back').click();
    await until(() => shown('home') && card());
    card().querySelector('.check-box').click();
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

    // Theme. The choice is put back afterwards, since the browser storage is
    // not inside the test folder.
    const themeBefore = localStorage.getItem('tacknote.theme');
    const background = () => getComputedStyle(document.body).backgroundColor;
    const frameTheme = () => window.__TAURI__.window.getCurrentWindow().theme();
    try {
      $('#theme-choice input[value="dark"]').click();
      check(background() === 'rgb(26, 25, 23)', 'choosing dark turns the page dark');
      check(await until(async () => (await frameTheme()) === 'dark'), 'the window frame turns dark too');
      $('#theme-choice input[value="light"]').click();
      check(background() === 'rgb(246, 244, 238)', 'choosing light turns the page light');
      check(await until(async () => (await frameTheme()) === 'light'), 'the window frame turns light too');
      check(localStorage.getItem('tacknote.theme') === 'light', 'the choice is remembered');
      $('#theme-choice input[value="system"]').click();
      const systemDark = matchMedia('(prefers-color-scheme: dark)').matches;
      check(background() === (systemDark ? 'rgb(26, 25, 23)' : 'rgb(246, 244, 238)'), 'following the system again');
    } finally {
      if (themeBefore === null) localStorage.removeItem('tacknote.theme');
      else localStorage.setItem('tacknote.theme', themeBefore);
    }
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

    // Sticky note windows on the desktop.
    const stickyWindow = async (noteId) => (await invoke('selftest_windows')).find((w) => w.label === `sticky-${noteId}`);
    const noteId = () => decodeURIComponent(location.hash.split('/')[2]);
    $('#settings-back').click();
    await until(() => shown('home'));
    cards()[0].click();
    check(await until(() => shown('editor')), 'the remaining note opens');
    const stuck = noteId();
    $('#stick-note').click();
    let sticky = await askSticky(stuck, 'state');
    check(sticky.value === box().value, 'sticking a note opens a window with its text');
    const info = await stickyWindow(stuck);
    check(info?.onTop && !info.decorated, 'the sticky window is frameless and stays on top');
    sticky = await askSticky(stuck, 'type', { text: `${sticky.value}\n便签窗口里写的` });
    check(await until(() => box().value.endsWith('\n便签窗口里写的')), 'typing in the sticky window shows in the open editor');
    fill(box(), `${box().value}!`);
    check(
      await until(async () => (await askSticky(stuck, 'state')).value.endsWith('便签窗口里写的!')),
      'typing in the editor shows in the sticky window',
    );
    await invoke('selftest_remote_edit', { id: stuck, text: '来自手机的改动' });
    check(
      await until(async () => (await askSticky(stuck, 'state')).value === '来自手机的改动'),
      'a change from a sync shows in the sticky window',
    );
    $('#colors [data-color="4"]').click();
    check(await until(async () => (await askSticky(stuck, 'state')).colorClass === 'color-4'), 'the sticky window takes the new colour');
    $('#theme-choice input[value="dark"]').click();
    check(await until(async () => (await askSticky(stuck, 'state')).theme === 'dark'), 'the sticky window follows the theme');
    $('#theme-choice input[value="system"]').click();
    await askSticky(stuck, 'toggleOnTop');
    check(await until(async () => (await stickyWindow(stuck))?.onTop === false), 'the sticky window can stop staying on top');
    await invoke('selftest_move_window', { label: `sticky-${stuck}`, x: 321, y: 234 });
    check(
      await until(async () => {
        const saved = (await invoke('selftest_layout')).find((s) => s.noteId === stuck);
        return saved && Math.round(saved.x) === 321 && Math.round(saved.y) === 234 && saved.onTop === false;
      }),
      'its place and the on-top choice are saved on this device',
    );
    await askSticky(stuck, 'close');
    check(await until(async () => !(await stickyWindow(stuck))), 'the close button takes it off the desktop');
    check(!(await invoke('selftest_layout')).some((s) => s.noteId === stuck), 'and it does not come back next time');
    $('#stick-note').click();
    check(await until(async () => !!(await stickyWindow(stuck))), 'it can be stuck again');
    // Loaded first: a window still loading would close itself on finding no note.
    await askSticky(stuck, 'state');
    $('#delete-note').click();
    check(await until(async () => !(await stickyWindow(stuck))), 'deleting a stuck note closes its window');

    // A new note stuck before anything is written in it is kept, and comes
    // back after a restart (checked by the "restore" self-test).
    await until(() => shown('home'));
    $('#new-note').click();
    await until(() => shown('editor'));
    const kept = noteId();
    $('#stick-note').click();
    await askSticky(kept, 'state');
    $('#editor-back').click();
    check(
      await until(async () => shown('home') && (await invoke('list_notes')).some((n) => n.id === kept)),
      'an empty note on the desktop is not discarded',
    );
    await askSticky(kept, 'type', { text: '贴在桌面上' });
    check(await until(async () => (await saved(kept)) === '贴在桌面上'), 'typing in a new sticky window saves the note');

    // Dragging cards into a new order, with the mouse as a person would.
    const made = [];
    for (const text of ['甲', '乙', '丙']) {
      const note = await invoke('create_note', {});
      await invoke('edit_text', { id: note.id, base: '', text });
      made.push(note.id);
    }
    const [jia, yi, bing] = made;
    const cardOf = (noteId) => $(`.card[data-id="${noteId}"]`);
    check(await until(() => cardOf(jia) && cardOf(yi) && cardOf(bing)), 'three new notes in the list');
    const listed = async () => (await invoke('list_notes')).map((n) => n.id).filter((noteId) => made.includes(noteId));
    check((await listed()).join() === [bing, yi, jia].join(), 'the newest comes first before any dragging');
    async function dragCard(from, to, lowerHalf) {
      const start = from.getBoundingClientRect();
      const end = to.getBoundingClientRect();
      const [x0, y0] = [start.left + start.width / 2, start.top + start.height / 2];
      const [x1, y1] = [end.left + end.width / 2, lowerHalf ? end.bottom - 4 : end.top + 4];
      from.dispatchEvent(new MouseEvent('mousedown', { bubbles: true, button: 0, clientX: x0, clientY: y0 }));
      for (let step = 1; step <= 6; step++) {
        const x = x0 + ((x1 - x0) * step) / 6;
        const y = y0 + ((y1 - y0) * step) / 6;
        window.dispatchEvent(new MouseEvent('mousemove', { bubbles: true, clientX: x, clientY: y }));
        await wait(20);
      }
      // Cards slide while dragging, so follow the target to where it is now.
      for (let round = 0; round < 3; round++) {
        await wait(250);
        const now = to.getBoundingClientRect();
        const y = lowerHalf ? now.bottom - 4 : now.top + 4;
        window.dispatchEvent(new MouseEvent('mousemove', { bubbles: true, clientX: now.left + now.width / 2, clientY: y }));
      }
      await wait(250);
      const preview = [...document.querySelectorAll('.card:not(.drag-ghost)')].map((el) => el.dataset.id);
      window.dispatchEvent(new MouseEvent('mouseup', { bubbles: true }));
      settling.push(document.querySelector('.drag-ghost')?.getAnimations().length > 0);
      return preview.filter((noteId) => made.includes(noteId));
    }
    const settling = [];
    const preview = await dragCard(cardOf(jia), cardOf(bing), false);
    // With reduced motion (as on CI machines) the card goes straight there.
    if (matchMedia('(prefers-reduced-motion: reduce)').matches) {
      check(settling[0] === false, 'with reduced motion the card goes straight to its new place when let go');
    } else {
      check(settling[0] === true, 'the card glides into its new place when let go');
    }
    check(preview.join() === [jia, bing, yi].join(), 'the other cards move aside while dragging, before letting go');
    check(
      await until(async () => (await listed()).join() === [jia, bing, yi].join()),
      'dragging the bottom card onto the top one puts it first',
    );
    check(shown('home') && !shown('editor'), 'a drag does not open the note');
    check(await until(() => {
      const order = [...document.querySelectorAll('.card')].map((el) => el.dataset.id).filter((noteId) => made.includes(noteId));
      return order.join() === [jia, bing, yi].join();
    }), 'the list shows the new order');
    await invoke('update_note', { id: bing, pinned: true });
    await until(() => cardOf(bing)?.querySelector('.pin-mark'));
    // Jia sits next to the pinned card, so the drag crosses no other card.
    await dragCard(cardOf(jia), cardOf(bing), false);
    await wait(300);
    check((await listed()).join() === [bing, jia, yi].join(), 'a card cannot be dragged among the pinned notes');

    // Not waiting for the save: quitting must write the new place itself.
    await invoke('selftest_move_window', { label: `sticky-${kept}`, x: 400, y: 300 });
  } catch (error) {
    check(false, `stopped early: ${error?.detail ?? error}`);
  } finally {
    if (themeBefore === null) localStorage.removeItem('tacknote.theme');
    else localStorage.setItem('tacknote.theme', themeBefore);
  }
  await finish();
}
