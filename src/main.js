import { icon } from './icons.js';
import {
  CARD_MAX_LINES,
  COLOR_COUNT,
  EditorSync,
  caretAfterReplace,
  colorClass,
  columnCountFor,
  distribute,
  insertCheckbox,
  matches,
  normalTheme,
} from './logic.js';
import { lang, t } from './strings.js';

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;

const $ = (id) => document.getElementById(id);
const state = { notes: [], query: '', sync: null, editor: null, fromHome: false };

// ---- Shared bits ----

document.documentElement.lang = lang;
for (const el of document.querySelectorAll('[data-text]')) el.textContent = t[el.dataset.text];
for (const el of document.querySelectorAll('[data-placeholder]')) el.placeholder = t[el.dataset.placeholder];
for (const el of document.querySelectorAll('[data-label]')) setLabel(el, t[el.dataset.label]);
for (const el of document.querySelectorAll('[data-icon]')) el.innerHTML = icon(el.dataset.icon);
$('empty-body').textContent = t.emptyBody;

function setLabel(el, label) {
  el.title = label;
  el.setAttribute('aria-label', label);
}

let toastTimer;
function toast(message) {
  const el = $('toast');
  el.textContent = message;
  el.classList.add('shown');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.classList.remove('shown'), 4000);
}

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

/**
 * On Android a cold-started page can run before Rust accepts calls, and such
 * a call is refused rather than queued. Retries for 3 seconds; errors from
 * the app itself are not retried.
 */
async function retry(action) {
  for (let attempt = 0; ; attempt++) {
    try {
      return await action();
    } catch (error) {
      if (error?.kind || attempt >= 30) throw error;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
  }
}

// ---- Theme: follow the system, light or dark ----

// Kept on this device only; the page's head applies it before the first paint.
const THEME_KEY = 'tacknote.theme';

function savedTheme() {
  try {
    return normalTheme(localStorage.getItem(THEME_KEY));
  } catch {
    return 'system';
  }
}

function applyTheme(theme) {
  document.documentElement.dataset.theme = theme;
  for (const input of document.querySelectorAll('#theme-choice input')) input.checked = input.value === theme;
  // The window frame follows too; phones have none, so a refusal is fine.
  try {
    window.__TAURI__.window.getCurrentWindow().setTheme(theme === 'system' ? null : theme).catch(() => {});
  } catch {}
}

$('theme-choice').addEventListener('change', (event) => {
  const theme = normalTheme(event.target.value);
  try {
    localStorage.setItem(THEME_KEY, theme);
  } catch (error) {
    console.error(error);
  }
  applyTheme(theme);
});

applyTheme(savedTheme());

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
  const notes = state.notes.filter((note) => matches(note, state.query));
  const searching = state.query.trim() !== '';
  $('empty').hidden = notes.length > 0;
  $('empty-title').textContent = searching ? t.noResults : t.emptyTitle;
  $('empty-body').hidden = searching;
  const container = $('notes');
  const scroll = container.scrollTop;
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
      row.className = line.checked ? 'check-line checked' : 'check-line';
      row.dataset.line = index;
      row.tabIndex = 0;
      row.setAttribute('role', 'checkbox');
      row.setAttribute('aria-checked', String(line.checked));
      row.innerHTML = icon(line.checked ? 'checked' : 'unchecked');
      row.append(Object.assign(document.createElement('span'), { textContent: line.text }));
    }
    el.append(row);
  });
  if (note.lines.length > CARD_MAX_LINES) el.append('…');
  return el;
}

function activateCard(target) {
  const line = target.closest('.check-line');
  const found = target.closest('.card');
  if (!found) return;
  if (line) {
    invoke('toggle_checkbox', { id: found.dataset.id, line: Number(line.dataset.line) }).catch(reportError);
  } else {
    navigate(`#/note/${encodeURIComponent(found.dataset.id)}`);
  }
}

$('notes').addEventListener('click', (event) => activateCard(event.target));
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

/** Shows a change from elsewhere without moving the caret out of place. */
function replaceText(text) {
  const start = caretAfterReplace(box.value, text, box.selectionStart);
  const end = caretAfterReplace(box.value, text, box.selectionEnd);
  const scroll = box.scrollTop;
  box.value = text;
  box.setSelectionRange(start, end);
  box.scrollTop = scroll;
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
      setValue: replaceText,
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

$('add-checkbox').addEventListener('click', () => {
  const { text, caret } = insertCheckbox(box.value, box.selectionStart);
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
  if (await invoke('selftest_enabled_cmd')) {
    const { run } = await import('./selftest.js');
    run({ invoke });
  }
}

start();
