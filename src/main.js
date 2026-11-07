import { applyTheme, fillPage, isPhone, replaceText, retry, saveTheme, savedTheme, setLabel, toast } from './common.js';
import { icon } from './icons.js';
import {
  CARD_MAX_LINES,
  COLOR_COUNT,
  EditorSync,
  colorClass,
  columnCountFor,
  distribute,
  dropBefore,
  moveId,
  toggleTodo,
  matches,
  normalTheme,
} from './logic.js';
import { lang, t } from './strings.js';

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
const state = { notes: [], query: '', sync: null, editor: null, fromHome: false };

// ---- Shared bits ----

fillPage();
$('empty-body').textContent = t.emptyBody;

function showFatal(detail) {
  $('fatal-detail').textContent = detail;
  $('fatal').hidden = false;
}

/** Rust errors arrive as { kind, detail }. A failed write is always told. */
function reportError(error) {
  console.error(error);
  if (error?.kind === 'startup') showFatal(error.detail);
  else toast(t.saveFailed);
}

// ---- Theme: follow the system, light or dark ----

function showTheme(theme) {
  applyTheme(theme);
  for (const input of document.querySelectorAll('#theme-choice input')) input.checked = input.value === theme;
}

$('theme-choice').addEventListener('change', (event) => {
  const theme = normalTheme(event.target.value);
  saveTheme(theme);
  showTheme(theme);
  window.__TAURI__.event.emit('theme-changed', theme).catch(console.error);
});

showTheme(savedTheme());

// ---- Pages ----

function navigate(hash) {
  state.fromHome = true;
  location.hash = hash;
}

/** Back to the list; the system back gesture on Android does the same. */
function goBack() {
  if (state.fromHome) history.back();
  else location.replace('#/');
}

function route() {
  const [, page, id] = location.hash.split('/');
  if (page !== 'note') closeEditor();
  if (page !== 'settings') $('webdav-password').value = '';
  if (page !== 'note' && page !== 'settings') state.fromHome = false;
  $('home').hidden = page === 'note' || page === 'settings';
  $('editor').hidden = page !== 'note';
  $('settings').hidden = page !== 'settings';
  if (page === 'note') openEditor(decodeURIComponent(id ?? ''));
  else if (page === 'settings') openSettings();
  else renderHome();
}

window.addEventListener('hashchange', route);
document.addEventListener('keydown', (event) => {
  if (event.key === 'Escape' && $('home').hidden) goBack();
});

// ---- Home: the list of notes ----

async function loadNotes() {
  state.notes = await invoke('list_notes');
  if (!$('home').hidden) renderHome();
}

function renderHome() {
  // Redrawing would pull the card out from under the finger.
  if (drag?.started) {
    drag.redraw = true;
    return;
  }
  const notes = state.notes.filter((note) => matches(note, state.query));
  const searching = state.query.trim() !== '';
  $('empty').hidden = notes.length > 0;
  $('empty-title').textContent = searching ? t.noResults : t.emptyTitle;
  $('empty-body').hidden = searching;
  const container = $('notes');
  const scroll = container.scrollTop;
  // The list is not redrawn while hidden, so these are the places the user
  // last saw, even when coming back from the editor.
  const was = cardPlaces();
  const columns = distribute(notes, columnCountFor(container.clientWidth || window.innerWidth));
  container.replaceChildren(
    ...columns.map((column) => {
      const el = document.createElement('div');
      el.className = 'column';
      el.append(...column.map(card));
      return el;
    }),
  );
  container.scrollTop = scroll;
  slideCards(was, true);
}

// ---- Card motion ----
// Whenever cards change places they slide there, so the user can follow
// where each one went; new cards fade in. None of it with reduced motion.

const calm = matchMedia('(prefers-reduced-motion: reduce)');
const SLIDE_MS = 220;

function cardPlaces() {
  return new Map([...$('notes').querySelectorAll('.card')].map((el) => [el.dataset.id, el.getBoundingClientRect()]));
}

/** Slides each card from where it was to where it is now. */
function slideCards(was, fadeInNew) {
  // Nothing was on screen before, such as at start: just show the list.
  if (calm.matches || ![...was.values()].some((place) => place.width > 0)) return;
  for (const el of $('notes').querySelectorAll('.card')) {
    const before = was.get(el.dataset.id);
    if (!before) {
      if (fadeInNew) {
        el.animate([{ opacity: 0, transform: 'scale(0.96)' }, { opacity: 1, transform: 'none' }], { duration: SLIDE_MS, easing: 'ease-out' });
      }
      continue;
    }
    const now = el.getBoundingClientRect();
    const dx = before.left - now.left;
    const dy = before.top - now.top;
    if (Math.abs(dx) < 1 && Math.abs(dy) < 1) continue;
    el.animate([{ transform: `translate(${dx}px, ${dy}px)` }, { transform: 'none' }], { duration: SLIDE_MS, easing: 'ease' });
  }
}

function card(note) {
  const el = document.createElement('article');
  el.className = `card ${colorClass(note.colorIndex)}`;
  el.dataset.id = note.id;
  el.tabIndex = 0;
  el.setAttribute('role', 'button');
  if (note.pinned || note.conflictCopy) {
    const tags = document.createElement('div');
    tags.className = 'card-tags';
    if (note.pinned) tags.insertAdjacentHTML('beforeend', `<span class="pin-mark">${icon('pin')}</span>`);
    if (note.conflictCopy) tags.append(Object.assign(document.createElement('span'), { textContent: t.conflictCopy }));
    el.append(tags);
  }
  note.lines.slice(0, CARD_MAX_LINES).forEach((line, index) => {
    const row = document.createElement('div');
    if (line.checked === null) {
      row.className = 'card-line';
      row.textContent = line.text;
    } else {
      // Only the box ticks; the text opens the note like the rest of the card.
      row.className = line.checked ? 'check-line checked' : 'check-line';
      const box = document.createElement('button');
      box.type = 'button';
      box.className = 'check-box';
      box.dataset.line = index;
      box.setAttribute('role', 'checkbox');
      box.setAttribute('aria-checked', String(line.checked));
      box.setAttribute('aria-label', line.text);
      box.innerHTML = icon(line.checked ? 'checked' : 'unchecked');
      row.append(box, Object.assign(document.createElement('span'), { textContent: line.text }));
    }
    el.append(row);
  });
  if (note.lines.length > CARD_MAX_LINES) el.append('…');
  return el;
}

function activateCard(target) {
  const line = target.closest('.check-box');
  const found = target.closest('.card');
  if (!found) return;
  if (line) {
    invoke('toggle_checkbox', { id: found.dataset.id, line: Number(line.dataset.line) }).catch(reportError);
  } else {
    navigate(`#/note/${encodeURIComponent(found.dataset.id)}`);
  }
}

$('notes').addEventListener('click', (event) => {
  if (draggedJustNow) {
    draggedJustNow = false;
    return;
  }
  activateCard(event.target);
});
$('notes').addEventListener('keydown', (event) => {
  if (event.key !== 'Enter' && event.key !== ' ') return;
  event.preventDefault();
  activateCard(event.target);
});

$('search').addEventListener('input', (event) => {
  state.query = event.target.value;
  renderHome();
});

let columnCount = 0;
window.addEventListener('resize', () => {
  const count = columnCountFor($('notes').clientWidth || window.innerWidth);
  if (count !== columnCount && !$('home').hidden) renderHome();
  columnCount = count;
});

// ---- Dragging cards to reorder ----
// A mouse drags once it moves a little; a finger holds still first, so a
// swipe still scrolls the list.

const HOLD_TO_DRAG = 450;
const SLOP = 8;
let drag = null;
let draggedJustNow = false;

function pickUp(target, x, y, touch) {
  if (state.query.trim() !== '' || target.closest('.check-box')) return null;
  const card = target.closest('.card');
  return card ? { id: card.dataset.id, card, startX: x, startY: y, touch, started: false } : null;
}

function startDrag(current) {
  current.started = true;
  current.order = state.notes.map((note) => note.id);
  current.settleUntil = 0;
  const rect = current.card.getBoundingClientRect();
  current.offsetX = current.startX - rect.left;
  current.offsetY = current.startY - rect.top;
  current.ghost = current.card.cloneNode(true);
  current.ghost.classList.add('drag-ghost');
  current.ghost.style.width = `${rect.width}px`;
  document.body.append(current.ghost);
  current.card.classList.add('dragging');
  moveGhost(current, current.startX, current.startY);
  navigator.vibrate?.(15);
}

/** The card in hand is a little bigger, so it reads as picked up. */
function moveGhost(current, x, y) {
  current.ghost.style.transform = `translate(${x - current.offsetX}px, ${y - current.offsetY}px) scale(1.03)`;
}

/** Lays the cards out in `order`, sliding each from where it was. */
function layoutCards(order) {
  const container = $('notes');
  const cards = new Map([...container.querySelectorAll('.card')].map((el) => [el.dataset.id, el]));
  const was = cardPlaces();
  const byId = new Map(state.notes.map((note) => [note.id, note]));
  const notes = order.map((id) => byId.get(id)).filter(Boolean);
  const scroll = container.scrollTop;
  container.replaceChildren(
    ...distribute(notes, columnCountFor(container.clientWidth || window.innerWidth)).map((column) => {
      const el = document.createElement('div');
      el.className = 'column';
      el.append(...column.map((note) => cards.get(note.id)).filter(Boolean));
      return el;
    }),
  );
  container.scrollTop = scroll;
  slideCards(was, false);
}

/** Every card moves to where it would be, as soon as the finger is over another card. */
function dragTo(current, x, y) {
  moveGhost(current, x, y);
  const list = $('notes');
  const box = list.getBoundingClientRect();
  if (y < box.top + 48) list.scrollTop -= 12;
  else if (y > box.bottom - 48) list.scrollTop += 12;
  // Cards still sliding are not where they will be; wait for them.
  if (performance.now() < current.settleUntil) return;
  const under = document.elementFromPoint(x, y)?.closest('.card');
  if (!under || under === current.card || under.classList.contains('drag-ghost')) return;
  const rect = under.getBoundingClientRect();
  const byId = new Map(state.notes.map((note) => [note.id, note]));
  const notes = current.order.map((id) => byId.get(id));
  const before = dropBefore(notes, current.id, under.dataset.id, y > rect.top + rect.height / 2);
  if (before === undefined) return;
  const order = moveId(current.order, current.id, before);
  if (order.join() === current.order.join()) return;
  current.order = order;
  current.settleUntil = performance.now() + 200;
  layoutCards(order);
}

function endDrag(current, drop) {
  clearTimeout(current.timer);
  if (!current.started) return;
  current.started = false;
  draggedJustNow = true;
  setTimeout(() => { draggedJustNow = false; }, 400);
  const changed = current.order.join() !== state.notes.map((note) => note.id).join();
  const finish = () => {
    current.ghost.remove();
    current.card.classList.remove('dragging');
    if (drop && changed) {
      const before = current.order[current.order.indexOf(current.id) + 1] ?? null;
      invoke('move_note', { id: current.id, before }).catch(reportError);
    } else if (changed || current.redraw) {
      renderHome();
    }
  };
  if (!drop || calm.matches) {
    finish();
    return;
  }
  // The card settles into its new place instead of vanishing.
  const place = current.card.getBoundingClientRect();
  current.ghost.animate(
    [{ transform: current.ghost.style.transform }, { transform: `translate(${place.left}px, ${place.top}px)` }],
    { duration: 160, easing: 'ease-out', fill: 'forwards' },
  ).onfinish = finish;
}

$('notes').addEventListener('mousedown', (event) => {
  if (event.button === 0) drag = pickUp(event.target, event.clientX, event.clientY, false);
});
window.addEventListener('mousemove', (event) => {
  if (!drag || drag.touch) return;
  if (!drag.started) {
    if (Math.hypot(event.clientX - drag.startX, event.clientY - drag.startY) < SLOP) return;
    startDrag(drag);
  }
  event.preventDefault();
  dragTo(drag, event.clientX, event.clientY);
});
window.addEventListener('mouseup', () => {
  if (!drag || drag.touch) return;
  endDrag(drag, true);
  drag = null;
});

$('notes').addEventListener('touchstart', (event) => {
  if (event.touches.length !== 1) return;
  const touch = event.touches[0];
  drag = pickUp(event.target, touch.clientX, touch.clientY, true);
  if (!drag) return;
  const current = drag;
  current.timer = setTimeout(() => {
    if (drag === current) startDrag(current);
  }, HOLD_TO_DRAG);
}, { passive: true });
$('notes').addEventListener('touchmove', (event) => {
  if (!drag?.touch) return;
  const touch = event.touches[0];
  if (!drag.started) {
    // Moved before the hold finished: this is a scroll.
    if (Math.hypot(touch.clientX - drag.startX, touch.clientY - drag.startY) > SLOP) {
      clearTimeout(drag.timer);
      drag = null;
    }
    return;
  }
  event.preventDefault();
  dragTo(drag, touch.clientX, touch.clientY);
}, { passive: false });
for (const type of ['touchend', 'touchcancel']) {
  $('notes').addEventListener(type, (event) => {
    if (!drag?.touch) return;
    if (drag.started) event.preventDefault();
    endDrag(drag, type === 'touchend');
    drag = null;
  });
}
// A long press would otherwise open the text menu on phones.
$('notes').addEventListener('contextmenu', (event) => {
  if (event.target.closest('.card')) event.preventDefault();
});

$('new-note').addEventListener('click', async () => {
  try {
    const note = await invoke('create_note', {});
    navigate(`#/note/${encodeURIComponent(note.id)}`);
  } catch (error) {
    reportError(error);
  }
});

$('sync-button').addEventListener('click', () => navigate('#/settings'));
$('settings-button').addEventListener('click', () => navigate('#/settings'));

// ---- Editor ----

const box = $('note-text');

for (let index = 0; index < COLOR_COUNT; index++) {
  const swatch = document.createElement('button');
  swatch.type = 'button';
  swatch.className = `swatch ${colorClass(index)}`;
  swatch.dataset.color = index;
  setLabel(swatch, t.color(index + 1));
  $('colors').append(swatch);
}

function showEditorChrome(note) {
  state.editor.note = note;
  $('editor').className = `view ${colorClass(note.colorIndex)}`;
  $('toggle-pin').innerHTML = icon(note.pinned ? 'pin' : 'pinOutline');
  setLabel($('toggle-pin'), note.pinned ? t.unpin : t.pin);
  $('editor-conflict').hidden = !note.conflictCopy;
  for (const swatch of $('colors').children) {
    swatch.setAttribute('aria-pressed', String(Number(swatch.dataset.color) === note.colorIndex));
  }
}

async function openEditor(id) {
  if (state.editor?.id === id) return;
  closeEditor();
  const hash = location.hash;
  let note = null;
  try {
    note = await invoke('get_note', { id });
  } catch (error) {
    reportError(error);
  }
  if (location.hash !== hash || state.editor) return;
  if (!note) {
    location.replace('#/');
    return;
  }
  state.editor = {
    id,
    note,
    sync: new EditorSync({
      base: note.text,
      save: (text, base) => invoke('edit_text', { id, base, text }),
      load: async () => (await invoke('get_note', { id }))?.text ?? null,
      getValue: () => box.value,
      setValue: (text) => replaceText(box, text),
      onError: reportError,
    }),
  };
  box.value = note.text;
  box.scrollTop = 0;
  showEditorChrome(note);
  if (note.text === '') box.focus();
}

/** A note left empty is not kept; Rust checks that under its own lock. */
function closeEditor() {
  const editor = state.editor;
  if (!editor) return;
  state.editor = null;
  editor.sync.close();
  if (!editor.deleted) invoke('discard_if_empty', { id: editor.id }).catch(reportError);
}

async function refreshEditor() {
  const editor = state.editor;
  if (!editor) return;
  editor.sync.refresh();
  const note = await invoke('get_note', { id: editor.id });
  if (note && state.editor === editor) showEditorChrome(note);
}

box.addEventListener('input', (event) => {
  if (!event.isComposing) state.editor?.sync.typed();
});
box.addEventListener('compositionstart', () => state.editor?.sync.compositionStarted());
box.addEventListener('compositionend', () => state.editor?.sync.compositionEnded());

$('editor-back').addEventListener('click', goBack);

// Phones have no desktop: there the button puts the note on the home screen
// widget instead.
if (isPhone) {
  $('stick-note').innerHTML = icon('widget');
  setLabel($('stick-note'), t.showInWidget);
}
$('stick-note').addEventListener('click', async () => {
  const editor = state.editor;
  if (!editor) return;
  try {
    if (!isPhone) {
      await invoke('stick_note', { id: editor.id });
    } else if (!(await invoke('show_in_widget', { id: editor.id }))) {
      // This launcher cannot add a widget for the app; the user adds one.
      toast(t.addWidgetByHand);
    }
  } catch (error) {
    console.error(error);
    toast(t.stickFailed);
  }
});

$('add-checkbox').addEventListener('click', () => {
  const { text, caret } = toggleTodo(box.value, box.selectionStart);
  box.value = text;
  box.focus();
  box.setSelectionRange(caret, caret);
  state.editor?.sync.typed();
});

$('toggle-pin').addEventListener('click', () => {
  const editor = state.editor;
  if (!editor) return;
  invoke('update_note', { id: editor.id, pinned: !editor.note.pinned }).catch(reportError);
});

$('colors').addEventListener('click', (event) => {
  const swatch = event.target.closest('.swatch');
  const editor = state.editor;
  if (!swatch || !editor) return;
  invoke('update_note', { id: editor.id, colorIndex: Number(swatch.dataset.color) }).catch(reportError);
});

$('delete-note').addEventListener('click', async () => {
  const editor = state.editor;
  if (!editor) return;
  try {
    await invoke('delete_note', { id: editor.id });
    editor.deleted = true;
    goBack();
  } catch (error) {
    reportError(error);
  }
});

// ---- Sync settings ----

const fields = ['webdav-url', 'webdav-username', 'webdav-password', 'save-settings'].map($);

async function openSettings() {
  fields.forEach((field) => { field.disabled = true; });
  $('settings-error').hidden = true;
  try {
    const settings = await invoke('get_settings');
    $('webdav-url').value = settings.url;
    $('webdav-username').value = settings.username;
    $('webdav-password').value = settings.password;
    $('password-unavailable').hidden = !settings.passwordUnavailable;
    checkUrl();
  } catch (error) {
    reportError(error);
  } finally {
    fields.forEach((field) => { field.disabled = false; });
  }
  showSyncStatus();
}

async function checkUrl() {
  const insecure = await invoke('url_is_insecure', { url: $('webdav-url').value });
  $('insecure-warning').hidden = !insecure;
}

$('webdav-url').addEventListener('input', checkUrl);
$('settings-back').addEventListener('click', goBack);

$('settings-form').addEventListener('submit', async (event) => {
  event.preventDefault();
  const button = $('save-settings');
  button.disabled = true;
  button.textContent = t.saving;
  $('settings-error').hidden = true;
  try {
    await invoke('save_settings', {
      url: $('webdav-url').value,
      username: $('webdav-username').value,
      password: $('webdav-password').value,
    });
    $('password-unavailable').hidden = true;
  } catch (error) {
    console.error(error);
    $('settings-error').textContent = error?.kind === 'keychain' ? t.keychainFailed : t.saveFailed;
    $('settings-error').hidden = false;
  } finally {
    button.disabled = false;
    button.textContent = t.saveAndSync;
  }
});

function showSyncStatus() {
  const sync = state.sync;
  if (!sync) return;
  let text = t.status(sync.status, sync.error);
  if (sync.lastSyncedAt && sync.status !== 'syncing') {
    const time = new Date(sync.lastSyncedAt).toLocaleString(lang === 'zh' ? 'zh-CN' : 'en');
    text += ` · ${t.lastSynced(time)}`;
  }
  $('settings-status').textContent = text;
}

function onSyncState(sync) {
  state.sync = sync;
  const button = $('sync-button');
  const name = { ok: 'cloudDone', syncing: 'sync', error: 'cloudOff' }[sync.status] ?? 'cloud';
  button.innerHTML = icon(name);
  button.classList.toggle('error', sync.status === 'error');
  button.classList.toggle('busy', sync.status === 'syncing');
  setLabel(button, t.status(sync.status, sync.error));
  showSyncStatus();
  if (sync.status === 'ok') {
    invoke('take_conflicts').then((count) => count > 0 && toast(t.conflicts(count)), reportError);
  }
}

// ---- Start ----

async function onNotesChanged() {
  try {
    await loadNotes();
    await refreshEditor();
  } catch (error) {
    reportError(error);
  }
}

async function start() {
  try {
    await retry(() => listen('notes-changed', onNotesChanged));
    await retry(() => listen('sync-state', (event) => onSyncState(event.payload)));
    state.notes = await retry(() => invoke('list_notes'));
    onSyncState(await invoke('sync_state'));
  } catch (error) {
    showFatal(String(error?.detail ?? error));
    return;
  }
  columnCount = columnCountFor($('notes').clientWidth || window.innerWidth);
  route();
  const selftest = await invoke('selftest_mode');
  if (selftest) {
    const { run } = await import('./selftest.js');
    run({ invoke, mode: selftest });
  }
}

start();
