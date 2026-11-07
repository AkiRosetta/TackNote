// The note editor: one line per row, a to-do shown as a box instead of
// `[ ]` or `[x]`. The note itself stays plain text with those marks.
//
// Every change that needs care (Enter, Backspace at a line start, paste,
// ticking a box) is worked out on the plain text by the functions in
// logic.js, then drawn again. Typing within a line is left to the browser.
// The element gets the same `value` and selection API as a textarea, so the
// rest of the page treats it like one.

import { icon } from './icons.js';
import { backspaceAt, enterAt, todoLines, todoText, toggleDone } from './logic.js';

const MARK = 4; // `[ ] ` and `[x] ` in the plain text

export function makeLineEditor(el, placeholder) {
  el.contentEditable = 'true';
  el.spellcheck = false;
  el.classList.add('line-editor');
  el.dataset.hint = placeholder;
  let lastCaret = [0, 0];
  let composing = false;

  function lineElement(line) {
    const row = document.createElement('div');
    row.className = 'ed-line';
    if (line.todo) {
      row.classList.add('todo');
      row.classList.toggle('checked', line.checked);
      const box = document.createElement('span');
      box.className = 'ed-box';
      box.contentEditable = 'false';
      box.innerHTML = icon(line.checked ? 'checked' : 'unchecked');
      row.append(box);
    }
    row.append(line.text || document.createElement('br'));
    return row;
  }

  function render(text) {
    el.replaceChildren(...todoLines(text).map(lineElement));
    el.classList.toggle('empty', text === '');
  }

  /** Text of a row without its box; nested rows and breaks become new lines. */
  function textOf(node) {
    let out = '';
    for (const child of node.childNodes) {
      if (child.nodeType === Node.TEXT_NODE) out += child.textContent;
      else if (child.nodeName === 'BR') out += '\n';
      else if (child.classList?.contains('ed-box')) continue;
      else if (child.nodeName === 'DIV' || child.nodeName === 'P') out += (out && !out.endsWith('\n') ? '\n' : '') + textOf(child);
      else out += textOf(child);
    }
    return out;
  }

  function readLines() {
    const lines = [];
    for (const node of el.childNodes) {
      if (node.nodeType === Node.TEXT_NODE) {
        if (node.textContent) lines.push({ todo: false, checked: false, text: node.textContent });
        continue;
      }
      if (node.nodeName === 'BR') {
        lines.push({ todo: false, checked: false, text: '' });
        continue;
      }
      if (node.nodeType !== Node.ELEMENT_NODE) continue;
      const box = node.querySelector(':scope > .ed-box');
      let text = textOf(node).replace(/ /g, ' ');
      if (text.endsWith('\n')) text = text.slice(0, -1);
      const [first, ...rest] = text.split('\n');
      lines.push({ todo: Boolean(box), checked: Boolean(box) && node.classList.contains('checked'), text: first });
      for (const extra of rest) lines.push({ todo: false, checked: false, text: extra });
    }
    return lines;
  }

  function read() {
    return todoText(readLines());
  }

  /** Whether the rows are exactly what the text would draw. */
  function regular() {
    const lines = todoLines(read());
    const rows = [...el.childNodes];
    return rows.length === lines.length && rows.every((row, i) =>
      row.nodeType === Node.ELEMENT_NODE &&
      row.classList.contains('ed-line') &&
      !row.querySelector('div, p') &&
      Boolean(row.querySelector(':scope > .ed-box')) === lines[i].todo &&
      (!lines[i].todo || row.firstChild.classList?.contains('ed-box')));
  }

  function lineStarts(text) {
    const starts = [0];
    for (let i = 0; i < text.length; i++) if (text[i] === '\n') starts.push(i + 1);
    return starts;
  }

  /** A DOM position as an offset in the plain text. */
  function offsetOf(node, offset) {
    const text = read();
    const starts = lineStarts(text);
    const lines = todoLines(text);
    const rows = [...el.childNodes];
    if (node === el) return offset >= rows.length ? text.length : starts[offset] + (lines[offset]?.todo ? MARK : 0);
    let row = node;
    while (row && row.parentNode !== el) row = row.parentNode;
    const index = rows.indexOf(row);
    if (index < 0) return lastCaret[0];
    const range = document.createRange();
    range.setStart(row, row.firstChild?.classList?.contains('ed-box') ? 1 : 0);
    range.setEnd(node, offset);
    const inner = Math.min(range.toString().length, lines[index]?.text.length ?? 0);
    return starts[index] + (lines[index]?.todo ? MARK : 0) + inner;
  }

  /** An offset in the plain text as a DOM position, never inside a box. */
  function pointAt(offset) {
    const text = read();
    const starts = lineStarts(text);
    const lines = todoLines(text);
    let index = starts.length - 1;
    while (index > 0 && starts[index] > offset) index--;
    const row = el.childNodes[index];
    if (!row) return [el, 0];
    let inner = Math.max(0, Math.min(offset - starts[index] - (lines[index].todo ? MARK : 0), lines[index].text.length));
    const walker = document.createTreeWalker(row, NodeFilter.SHOW_TEXT);
    for (let node = walker.nextNode(); node; node = walker.nextNode()) {
      if (node.parentElement.closest('.ed-box')) continue;
      if (inner <= node.length) return [node, inner];
      inner -= node.length;
    }
    const count = row.childNodes.length;
    return [row, row.lastChild?.nodeName === 'BR' ? count - 1 : count];
  }

  function caret() {
    const selection = getSelection();
    if (selection.rangeCount && el.contains(selection.anchorNode) && regular()) {
      const range = selection.getRangeAt(0);
      lastCaret = [offsetOf(range.startContainer, range.startOffset), offsetOf(range.endContainer, range.endOffset)];
    }
    return lastCaret;
  }

  function setCaret(start, end = start) {
    lastCaret = [start, end];
    // A textarea keeps its caret while not focused; this only moves the
    // page's selection when the editor has focus, so it never steals it.
    if (document.activeElement !== el) return;
    const range = document.createRange();
    range.setStart(...pointAt(start));
    range.setEnd(...pointAt(end));
    const selection = getSelection();
    selection.removeAllRanges();
    selection.addRange(range);
  }

  /** Draws the changed text and tells the page, as typing would. */
  function apply(text, at) {
    render(text);
    setCaret(at);
    el.dispatchEvent(new Event('input', { bubbles: true }));
  }

  el.addEventListener('beforeinput', (event) => {
    if (event.isComposing || composing) return;
    const type = event.inputType;
    if (type.startsWith('format')) {
      event.preventDefault();
      return;
    }
    let change = null;
    const [start, end] = caret();
    if (type === 'insertParagraph' || type === 'insertLineBreak') change = enterAt(read(), start, end);
    else if (type === 'deleteContentBackward' && start === end) change = backspaceAt(read(), start);
    if (!change) return;
    event.preventDefault();
    apply(change.text, change.caret);
  });

  // Plain text only, so pasted formatting never ends up in the note.
  el.addEventListener('paste', (event) => {
    event.preventDefault();
    const pasted = (event.clipboardData?.getData('text/plain') ?? '').replace(/\r\n?/g, '\n');
    const [start, end] = caret();
    const text = read();
    apply(text.slice(0, start) + pasted + text.slice(end), start + pasted.length);
  });

  el.addEventListener('compositionstart', () => { composing = true; });
  el.addEventListener('compositionend', () => { composing = false; });

  // Registered before the page's own listener, so the page reads tidy rows.
  el.addEventListener('input', (event) => {
    if (composing || event.isComposing) return;
    if (!regular()) {
      // Typing `[ ] ` by hand, or a delete across lines: draw it again.
      const [start, end] = caret();
      render(read());
      setCaret(start, end);
    }
    el.classList.toggle('empty', read() === '');
  });

  document.addEventListener('selectionchange', () => {
    if (el.contains(getSelection().anchorNode)) caret();
  });

  // Only the box ticks; tapping it does not move the caret.
  el.addEventListener('mousedown', (event) => {
    if (event.target.closest('.ed-box')) event.preventDefault();
  });
  el.addEventListener('click', (event) => {
    const box = event.target.closest('.ed-box');
    if (!box) return;
    const index = [...el.childNodes].indexOf(box.parentNode);
    apply(toggleDone(read(), index), lastCaret[0]);
  });

  Object.defineProperty(el, 'value', {
    get: read,
    set(text) {
      render(text);
      lastCaret = [text.length, text.length];
    },
  });
  Object.defineProperty(el, 'selectionStart', { get: () => caret()[0] });
  Object.defineProperty(el, 'selectionEnd', { get: () => caret()[1] });
  el.setSelectionRange = setCaret;
  render('');
  return el;
}
