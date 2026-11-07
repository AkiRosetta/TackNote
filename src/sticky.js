// A sticky note window: one note, edited in place. It refreshes whenever the
// note changes elsewhere, like the editor in the main window.

import { applyTheme, fillPage, replaceText, retry, savedTheme, setLabel, toast } from './common.js';
import { icon } from './icons.js';
import { makeLineEditor } from './editor.js';
import { EditorSync, colorClass } from './logic.js';
import { t } from './strings.js';

const { invoke } = window.__TAURI__.core;
const { listen } = window.__TAURI__.event;
const id = window.__TAURI__.window.getCurrentWindow().label.replace(/^sticky-/, '');

const box = makeLineEditor(document.getElementById('sticky-text'), t.placeholder);
const onTopButton = document.getElementById('keep-on-top');
const closeButton = document.getElementById('close-sticky');
let editor = null;
let onTop = true;

fillPage();
applyTheme(savedTheme());

function reportError(error) {
  console.error(error);
  toast(t.saveFailed);
}

function showOnTop(value) {
  onTop = value;
  onTopButton.innerHTML = icon(value ? 'pin' : 'pinOutline');
  onTopButton.setAttribute('aria-pressed', String(value));
  setLabel(onTopButton, value ? t.stopKeepOnTop : t.keepOnTop);
}

function showColor(note) {
  document.body.className = `sticky-page ${colorClass(note.colorIndex)}`;
}

async function refresh() {
  if (!editor) return;
  editor.refresh();
  const note = await invoke('get_note', { id });
  if (note) showColor(note);
}

box.addEventListener('input', (event) => {
  if (!event.isComposing) editor?.typed();
});
box.addEventListener('compositionstart', () => editor?.compositionStarted());
box.addEventListener('compositionend', () => editor?.compositionEnded());

onTopButton.addEventListener('click', async () => {
  try {
    await invoke('set_sticky_on_top', { id, onTop: !onTop });
    showOnTop(!onTop);
  } catch (error) {
    console.error(error);
  }
});

closeButton.addEventListener('click', () => invoke('unstick_note', { id }).catch(console.error));

async function start() {
  await retry(() => listen('notes-changed', () => refresh().catch(reportError)));
  await retry(() => listen('theme-changed', (event) => applyTheme(event.payload)));
  const note = await retry(() => invoke('get_note', { id }));
  if (!note || note.deleted) {
    invoke('unstick_note', { id });
    return;
  }
  editor = new EditorSync({
    base: note.text,
    save: (text, base) => invoke('edit_text', { id, base, text }),
    load: async () => (await invoke('get_note', { id }))?.text ?? null,
    getValue: () => box.value,
    setValue: (text) => replaceText(box, text),
    onError: reportError,
  });
  box.value = note.text;
  showColor(note);
  showOnTop((await invoke('sticky_on_top', { id })) ?? true);
  if (await invoke('selftest_mode')) {
    const { serveSticky } = await import('./selftest.js');
    serveSticky({ id, box, onTopButton, closeButton });
  }
}

start().catch(reportError);
