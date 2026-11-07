// Helpers shared by the main window and the sticky note windows.

import { icon } from './icons.js';
import { caretAfterReplace, normalTheme } from './logic.js';
import { lang, t } from './strings.js';

export const isPhone = /Android|iPhone|iPad/i.test(navigator.userAgent);

export function setLabel(el, label) {
  el.title = label;
  el.setAttribute('aria-label', label);
}

/** Fills in the text, labels and icons the page names in data- attributes. */
export function fillPage() {
  document.documentElement.lang = lang;
  for (const el of document.querySelectorAll('[data-text]')) el.textContent = t[el.dataset.text];
  for (const el of document.querySelectorAll('[data-placeholder]')) el.placeholder = t[el.dataset.placeholder];
  for (const el of document.querySelectorAll('[data-label]')) setLabel(el, t[el.dataset.label]);
  for (const el of document.querySelectorAll('[data-icon]')) el.innerHTML = icon(el.dataset.icon);
}

let toastTimer;
export function toast(message) {
  const el = document.getElementById('toast');
  el.textContent = message;
  el.classList.add('shown');
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.classList.remove('shown'), 4000);
}

/**
 * On Android a cold-started page can run before Rust accepts calls, and such
 * a call is refused rather than queued. Retries for 3 seconds; errors from
 * the app itself are not retried.
 */
export async function retry(action) {
  for (let attempt = 0; ; attempt++) {
    try {
      return await action();
    } catch (error) {
      if (error?.kind || attempt >= 30) throw error;
      await new Promise((resolve) => setTimeout(resolve, 100));
    }
  }
}

/** Shows a change from elsewhere without moving the caret out of place. */
export function replaceText(box, text) {
  const start = caretAfterReplace(box.value, text, box.selectionStart);
  const end = caretAfterReplace(box.value, text, box.selectionEnd);
  const scroll = box.scrollTop;
  box.value = text;
  box.setSelectionRange(start, end);
  box.scrollTop = scroll;
}

// Theme: kept on this device only. Each page's head applies it before the
// first paint; the main window tells the others when it changes.
const THEME_KEY = 'tacknote.theme';

export function savedTheme() {
  try {
    return normalTheme(localStorage.getItem(THEME_KEY));
  } catch {
    return 'system';
  }
}

export function saveTheme(theme) {
  try {
    localStorage.setItem(THEME_KEY, theme);
  } catch (error) {
    console.error(error);
  }
}

export function applyTheme(theme) {
  document.documentElement.dataset.theme = theme;
  // The window frame follows too; phones have none, so a refusal is fine.
  try {
    window.__TAURI__.window.getCurrentWindow().setTheme(theme === 'system' ? null : theme).catch(() => {});
  } catch {}
}
