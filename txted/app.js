'use strict';

const $ = id => document.getElementById(id);
const HAS_FS = 'showOpenFilePicker' in window;
const SKIP = new Set(['.git', 'node_modules', '.DS_Store', '__pycache__']);

// ---------- tiny IndexedDB key/value store ----------
const db = {
  _p: null,
  open() {
    return this._p ||= new Promise((res, rej) => {
      const r = indexedDB.open('txted', 1);
      r.onupgradeneeded = () => r.result.createObjectStore('kv');
      r.onsuccess = () => res(r.result);
      r.onerror = () => rej(r.error);
    });
  },
  async get(k) { const d = await this.open(); return new Promise(res => { const r = d.transaction('kv').objectStore('kv').get(k); r.onsuccess = () => res(r.result); r.onerror = () => res(undefined); }); },
  async set(k, v) { const d = await this.open(); return new Promise(res => { const t = d.transaction('kv', 'readwrite'); t.objectStore('kv').put(v, k); t.oncomplete = res; t.onerror = res; }); },
};

// ---------- state ----------
let monaco, editor;
let tabs = [];            // {id, name, handle, model, savedAlt, viewState}
let active = null;
let nextId = 1;
let folder = null;        // {handle, files: [{path, handle}]}
let untitledN = 0;

if ('serviceWorker' in navigator) navigator.serviceWorker.register('sw.js').catch(() => {});

require.config({ paths: { vs: 'vendor/vs' } });
require(['vs/editor/editor.main'], m => { monaco = m; boot(); });

function boot() {
  monaco.editor.defineTheme('monokai', {
    base: 'vs-dark', inherit: true,
    rules: [
      { token: 'comment', foreground: '75715e' },
      { token: 'string', foreground: 'e6db74' },
      { token: 'number', foreground: 'ae81ff' },
      { token: 'keyword', foreground: 'f92672' },
      { token: 'type', foreground: '66d9ef', fontStyle: 'italic' },
      { token: 'tag', foreground: 'f92672' },
      { token: 'attribute.name', foreground: 'a6e22e' },
      { token: 'delimiter', foreground: 'f8f8f2' },
    ],
    colors: {
      'editor.background': '#272822', 'editor.foreground': '#f8f8f2',
      'editor.lineHighlightBackground': '#3e3d32', 'editorCursor.foreground': '#f8f8f0',
      'editor.selectionBackground': '#49483e', 'editorLineNumber.foreground': '#90908a',
      'editorLineNumber.activeForeground': '#c2c2bf', 'editorWhitespace.foreground': '#464741',
      'menu.background': '#1e1f1c', 'menu.foreground': '#f8f8f2', 'menu.border': '#75715e',
      'menu.selectionBackground': '#49483e', 'menu.selectionForeground': '#ffffff', 'menu.separatorBackground': '#555',
      'editorWidget.background': '#1e1f1c', 'editorWidget.border': '#75715e',
      'editorSuggestWidget.background': '#1e1f1c', 'editorSuggestWidget.border': '#75715e', 'editorSuggestWidget.selectedBackground': '#49483e',
      'editorHoverWidget.background': '#1e1f1c', 'editorHoverWidget.border': '#75715e',
      'quickInput.background': '#1e1f1c', 'input.background': '#272822', 'widget.shadow': '#000000cc',
      'editorIndentGuide.background1': '#3b3a32', 'editorIndentGuide.activeBackground1': '#767771',
    },
  });
  editor = monaco.editor.create($('editor'), {
    model: null, theme: 'monokai', automaticLayout: true,
    fontFamily: 'ui-monospace, Menlo, Consolas, "DejaVu Sans Mono", monospace', fontSize: 14,
    minimap: { enabled: true }, multiCursorModifier: 'ctrlCmd', renderWhitespace: 'selection',
    scrollBeyondLastLine: true, smoothScrolling: true, cursorSmoothCaretAnimation: 'on',
    mouseWheelZoom: true, wordWrap: 'off', bracketPairColorization: { enabled: true },
    renderLineHighlight: 'all', fixedOverflowWidgets: true, useShadowDOM: false,
  });
  addCommands();
  wireUI();
  restoreSession();
}

// ---------- language ----------
function langFor(name) {
  const ext = '.' + (name.split('.').pop() || '').toLowerCase();
  const base = name.toLowerCase();
  for (const l of monaco.languages.getLanguages()) {
    if (l.extensions?.includes(ext) || l.filenames?.includes(name) || l.filenames?.includes(base)) return l.id;
  }
  return 'plaintext';
}

// ---------- tabs ----------
function addTab({ name, handle = null, text = '', dirty = false, lang, activate = true }) {
  const model = monaco.editor.createModel(text, lang || langFor(name));
  const t = { id: nextId++, name, handle, model, savedAlt: model.getAlternativeVersionId(), viewState: null, forceDirty: dirty };
  model.onDidChangeContent(() => { t.forceDirty = false; renderTabs(); scheduleSave(); });
  tabs.push(t);
  if (activate) activateTab(t); else renderTabs();
  return t;
}
const isDirty = t => t.forceDirty || t.model.getAlternativeVersionId() !== t.savedAlt;

function activateTab(t) {
  closeDiff();
  if (active) active.viewState = editor.saveViewState();
  active = t;
  editor.setModel(t.model);
  if (t.viewState) editor.restoreViewState(t.viewState);
  editor.focus();
  renderTabs(); markTree(); updateStatus(); scheduleSave();
}

async function closeTab(t) {
  if (isDirty(t) && !confirm(`"${t.name}" has unsaved changes. Close anyway?`)) return;
  if (diffSt && (diffSt.left.model === t.model || diffSt.right.model === t.model)) closeDiff();
  const i = tabs.indexOf(t);
  tabs.splice(i, 1);
  t.model.dispose();
  if (active === t) {
    active = null;
    if (tabs.length) activateTab(tabs[Math.min(i, tabs.length - 1)]);
    else { editor.setModel(null); renderTabs(); updateStatus(); markTree(); scheduleSave(); }
  } else { renderTabs(); scheduleSave(); }
}

function renderTabs() {
  const el = $('tabs');
  const plus = document.createElement('div');
  plus.className = 'tab plus'; plus.textContent = '+'; plus.title = 'New file (Ctrl+N)';
  plus.onclick = newTab;
  el.replaceChildren(...tabs.map(t => {
    const d = document.createElement('div');
    d.className = 'tab' + (t === active ? ' active' : '') + (isDirty(t) ? ' dirty' : '');
    d.title = t.handle ? t.name : t.name + ' (unsaved file)';
    d.innerHTML = '<span></span><span class="x"><span>×</span></span>';
    d.firstChild.textContent = t.name;
    d.onclick = () => activateTab(t);
    d.onauxclick = e => { if (e.button === 1) closeTab(t); };
    d.querySelector('.x').onclick = e => { e.stopPropagation(); closeTab(t); };
    return d;
  }), plus);
  el.querySelector('.active')?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
  document.title = (active ? (isDirty(active) ? '● ' : '') + active.name + ' — ' : '') + 'txted';
}

// ---------- files ----------
async function readFile(handle) {
  const f = await handle.getFile();
  return f.text();
}

async function openHandle(handle) {
  for (const t of tabs) if (t.handle && await t.handle.isSameEntry(handle)) return activateTab(t);
  addTab({ name: handle.name, handle, text: await readFile(handle) });
}

async function openFiles() {
  if (!HAS_FS) return fallbackOpen();
  try {
    const hs = await showOpenFilePicker({ multiple: true });
    for (const h of hs) await openHandle(h);
  } catch (e) { if (e.name !== 'AbortError') alert(e.message); }
}

function fallbackOpen() {
  const i = document.createElement('input');
  i.type = 'file'; i.multiple = true;
  i.onchange = async () => { for (const f of i.files) addTab({ name: f.name, text: await f.text() }); };
  i.click();
}

async function saveTab(t, as = false) {
  if (!t) return false;
  const text = t.model.getValue();
  if (HAS_FS) {
    try {
      if (!t.handle || as) {
        t.handle = await showSaveFilePicker({ suggestedName: t.name });
        t.name = t.handle.name;
        monaco.editor.setModelLanguage(t.model, langFor(t.name));
      }
      const w = await t.handle.createWritable();
      await w.write(text);
      await w.close();
    } catch (e) { if (e.name !== 'AbortError') alert('Save failed: ' + e.message); return false; }
  } else {
    const a = document.createElement('a');
    a.href = URL.createObjectURL(new Blob([text]));
    a.download = t.name; a.click(); URL.revokeObjectURL(a.href);
  }
  t.savedAlt = t.model.getAlternativeVersionId();
  t.forceDirty = false;
  renderTabs(); updateStatus(); scheduleSave();
  return true;
}

async function saveAll() { for (const t of tabs) if (isDirty(t)) await saveTab(t); }

const newTab = () => addTab({ name: untitledN++ ? `untitled-${untitledN}.txt` : 'untitled.txt' });

// ---------- folder / tree ----------
async function openFolder() {
  if (!HAS_FS) return alert('This browser has no File System Access API. Use Chrome or Edge, or in Brave enable brave://flags/#file-system-access-api and relaunch.');
  try { await setFolder(await showDirectoryPicker({ mode: 'readwrite' })); }
  catch (e) { if (e.name !== 'AbortError') alert(e.message); }
}

async function setFolder(handle) {
  folder = { handle, files: [] };
  $('folder-name').textContent = handle.name;
  db.set('folder', handle);
  const root = $('tree');
  root.replaceChildren();
  await renderDir(handle, root, 0);
  indexFiles(handle, '', folder);
}

async function listDir(handle) {
  const out = [];
  for await (const e of handle.values()) if (!SKIP.has(e.name)) out.push(e);
  return out.sort((a, b) => (a.kind === b.kind ? a.name.localeCompare(b.name, undefined, { numeric: true }) : a.kind === 'directory' ? -1 : 1));
}

async function renderDir(handle, container, depth) {
  for (const e of await listDir(handle)) {
    const row = document.createElement('div');
    row.className = 'row';
    row.style.paddingLeft = 8 + depth * 14 + 'px';
    const dir = e.kind === 'directory';
    row.innerHTML = `<span class="ic">${dir ? '▸' : ''}</span><span></span>`;
    row.lastChild.textContent = e.name;
    row._handle = e;
    container.appendChild(row);
    if (dir) {
      const kids = document.createElement('div');
      kids.hidden = true;
      container.appendChild(kids);
      let loaded = false;
      row.onclick = async () => {
        kids.hidden = !kids.hidden;
        row.firstChild.textContent = kids.hidden ? '▸' : '▾';
        if (!loaded) { loaded = true; await renderDir(e, kids, depth + 1); }
      };
    } else row.onclick = () => openHandle(e).catch(err => alert(err.message));
  }
}

async function indexFiles(dir, prefix, f) {
  let n = 0;
  async function walk(d, p) {
    for await (const e of d.values()) {
      if (SKIP.has(e.name) || f !== folder || n > 20000) continue;
      if (e.kind === 'directory') await walk(e, p + e.name + '/');
      else { f.files.push({ path: p + e.name, handle: e }); n++; }
    }
  }
  await walk(dir, prefix);
}

async function markTree() {
  const rows = $('tree').querySelectorAll('.row');
  for (const r of rows) {
    r.classList.toggle('active', !!(active?.handle && r._handle?.kind === 'file' && await r._handle.isSameEntry(active.handle).catch(() => false)));
  }
}

// ---------- quick open (Ctrl+P) ----------
function fuzzy(q, s) {
  q = q.toLowerCase(); const l = s.toLowerCase();
  let i = 0, score = 0, last = -2;
  for (const c of q) {
    const j = l.indexOf(c, i);
    if (j < 0) return -1;
    score += (j === last + 1 ? 3 : 1) + (j === 0 || '/._-'.includes(l[j - 1]) ? 2 : 0);
    last = j; i = j + 1;
  }
  return score - s.length * 0.01;
}

function quickOpen() {
  pick([
    ...tabs.map(t => ({ label: t.name, hint: 'open tab', run: () => activateTab(t) })),
    ...(folder?.files || []).map(f => ({ label: f.path, hint: '', run: () => openHandle(f.handle) })),
  ], 'Go to file…');
}

function setSyntax() {
  if (!active) return;
  const cur = active.model.getLanguageId();
  pick(monaco.languages.getLanguages()
    .map(l => ({ label: l.aliases?.[0] || l.id, hint: l.id === cur ? 'current' : l.id, run: () => { monaco.editor.setModelLanguage(active.model, l.id); updateStatus(); scheduleSave(); } }))
    .sort((a, b) => a.label.localeCompare(b.label)), 'Select syntax…');
}

function pick(items, placeholder) {
  const box = $('quick'), input = $('quick-input'), list = $('quick-list');
  let shown = [], sel = 0;
  const close = () => { box.hidden = true; input.oninput = input.onkeydown = null; editor.focus(); };
  const draw = () => {
    const q = input.value.trim();
    shown = (q ? items.map(i => ({ i, s: fuzzy(q, i.label) })).filter(x => x.s >= 0).sort((a, b) => b.s - a.s).map(x => x.i) : items).slice(0, 60);
    sel = Math.min(sel, Math.max(0, shown.length - 1));
    list.replaceChildren(...shown.map((it, n) => {
      const d = document.createElement('div');
      if (n === sel) d.className = 'sel';
      d.innerHTML = '<span></span><small></small>';
      d.firstChild.textContent = it.label; d.lastChild.textContent = it.hint;
      d.onclick = () => { close(); it.run(); };
      return d;
    }));
    list.querySelector('.sel')?.scrollIntoView({ block: 'nearest' });
  };
  input.placeholder = placeholder; input.value = ''; sel = 0; box.hidden = false; draw(); input.focus();
  input.oninput = () => { sel = 0; draw(); };
  input.onkeydown = e => {
    if (e.key === 'Escape') { e.preventDefault(); close(); }
    else if (e.key === 'ArrowDown') { e.preventDefault(); sel = Math.min(sel + 1, shown.length - 1); draw(); }
    else if (e.key === 'ArrowUp') { e.preventDefault(); sel = Math.max(sel - 1, 0); draw(); }
    else if (e.key === 'Enter') { e.preventDefault(); const it = shown[sel]; close(); it?.run(); }
  };
  input.onblur = () => setTimeout(() => { if (!box.hidden && document.activeElement !== input) close(); }, 100);
}

// ---------- commands / keys ----------
function addCommands() {
  const K = monaco.KeyCode, M = monaco.KeyMod;
  const add = (id, label, keys, run) => editor.addAction({ id, label, keybindings: keys, run });
  add('txted.open', 'File: Open…', [M.CtrlCmd | K.KeyO], openFiles);
  add('txted.openFolder', 'File: Open Folder…', [M.CtrlCmd | M.Shift | K.KeyO], openFolder);
  add('txted.new', 'File: New', [M.CtrlCmd | K.KeyN], newTab);
  add('txted.save', 'File: Save', [M.CtrlCmd | K.KeyS], () => saveTab(active));
  add('txted.saveAs', 'File: Save As…', [M.CtrlCmd | M.Shift | K.KeyS], () => saveTab(active, true));
  add('txted.saveAll', 'File: Save All', [M.CtrlCmd | M.Alt | K.KeyS], saveAll);
  add('txted.close', 'File: Close Tab', [M.CtrlCmd | K.KeyW, M.Alt | K.KeyW], () => active && closeTab(active));
  add('txted.quickOpen', 'Go to File…', [M.CtrlCmd | K.KeyP], quickOpen);
  add('txted.format', 'Edit: Format (document or selection)', [M.Shift | M.Alt | K.KeyF], formatText);
  add('txted.minify', 'Edit: Minify (document or selection)', [M.CtrlCmd | M.Shift | M.Alt | K.KeyM], minifyText);
  add('txted.sortLines', 'Lines: Sort (A→Z)', [K.F9], () => lineOp('Sort Lines (A→Z)'));
  add('txted.sortLinesDesc', 'Lines: Sort (Z→A)', [M.Shift | K.F9], () => lineOp('Sort Lines (Z→A)'));
  add('txted.uniqueLines', 'Lines: Unique (Keep First)', [], () => lineOp('Unique Lines (Keep First)'));
  add('txted.lineOps', 'Lines: All Sort / Unique Options…', [], () => pick(
    Object.keys(Transform.LINE_OPS).map(label => ({ label, hint: '', run: () => lineOp(label) })), 'Sort / unique lines…'));
  for (const label of Object.keys(Transform.LINE_OPS)) {
    if (/A→Z|Z→A|Keep First/.test(label)) continue;
    add('txted.line.' + label, 'Lines: ' + label.replace(/^(Sort|Unique) Lines/, '$1'), [], () => lineOp(label));
  }
  const CONVERT = [
    ['JSON → Java POJO', 'json', (t, n) => Convert.jsonToJava(t, n, 'pojo')],
    ['JSON → Java POJO (Lombok @Data)', 'json', (t, n) => Convert.jsonToJava(t, n, 'lombok')],
    ['JSON → Java record', 'json', (t, n) => Convert.jsonToJava(t, n, 'record')],
    ['XML → Java POJO (Jackson XML)', 'xml', (t, n) => Convert.xmlToJava(t, n, 'pojo')],
    ['XML → Java POJO (Lombok @Data)', 'xml', (t, n) => Convert.xmlToJava(t, n, 'lombok')],
    ['Java → JSON (sample)', 'json', (t) => Convert.javaToJson(t)],
    ['Java → XML (sample)', 'xml', (t) => Convert.javaToXml(t)],
  ];
  for (const [label, lang, fn] of CONVERT) add('txted.convert.' + label, 'Convert: ' + label, [], () => convert(label, lang, fn));
  add('txted.convertMenu', 'Convert: JSON / XML ↔ Java…', [M.CtrlCmd | M.Shift | K.KeyJ], () => pick(
    CONVERT.map(([label, lang, fn]) => ({ label, hint: '', run: () => convert(label, lang, fn) })), 'Convert…'));
  add('txted.cmpTab', 'Compare: Active Tab With Another Tab…', [], () => {
    if (!active) return;
    const others = tabs.filter(t => t !== active);
    if (!others.length) return toast('Open a second tab to compare with');
    pick(others.map(t => ({ label: t.name, hint: 'open tab', run: () => openDiff({ model: t.model, label: t.name }, { model: active.model, label: active.name }) })), 'Compare with…');
  });
  add('txted.cmpFile', 'Compare: With File on Disk…', [], async () => {
    if (!active) return;
    const f = await pickOneFile(); if (!f) return;
    openDiff({ model: monaco.editor.createModel(f.text, active.model.getLanguageId()), label: f.name, owned: true }, { model: active.model, label: active.name });
  });
  add('txted.cmpSaved', 'Compare: With Saved Version on Disk', [], async () => {
    if (!active?.handle) return toast('This tab is not backed by a file on disk');
    try {
      openDiff({ model: monaco.editor.createModel(await readFile(active.handle), active.model.getLanguageId()), label: active.name + ' (on disk)', owned: true }, { model: active.model, label: active.name + ' (editor)' });
    } catch (e) { toast('Could not read file: ' + e.message); }
  });
  add('txted.cmpClip', 'Compare: With Clipboard', [], async () => {
    if (!active) return;
    try {
      openDiff({ model: monaco.editor.createModel(await navigator.clipboard.readText(), active.model.getLanguageId()), label: 'Clipboard', owned: true }, { model: active.model, label: active.name });
    } catch { toast('Clipboard access was denied'); }
  });
  add('txted.cmpClose', 'Compare: Close Diff', [], closeDiff);
  add('txted.syntax', 'View: Set Syntax…', [], setSyntax);
  add('txted.palette', 'Command Palette', [M.CtrlCmd | M.Shift | K.KeyP], () => editor.trigger('kb', 'editor.action.quickCommand', null));
  add('txted.nextTab', 'View: Next Tab', [M.CtrlCmd | K.PageDown, M.Alt | K.PageDown], () => stepTab(1));
  add('txted.prevTab', 'View: Previous Tab', [M.CtrlCmd | K.PageUp, M.Alt | K.PageUp], () => stepTab(-1));
  add('txted.sidebar', 'View: Toggle Sidebar', [M.CtrlCmd | K.KeyB], () => { $('sidebar').hidden = $('splitter').hidden = !$('sidebar').hidden; });
  add('txted.minimap', 'View: Toggle Minimap', [], () => editor.updateOptions({ minimap: { enabled: !editor.getOption(monaco.editor.EditorOption.minimap).enabled } }));
  add('txted.wrap', 'View: Toggle Word Wrap', [M.Alt | K.KeyZ], () => editor.updateOptions({ wordWrap: editor.getOption(monaco.editor.EditorOption.wordWrap) === 'off' ? 'on' : 'off' }));
  add('txted.whitespace', 'View: Toggle Show Whitespace', [], () => editor.updateOptions({ renderWhitespace: editor.getOption(monaco.editor.EditorOption.renderWhitespace) === 'all' ? 'selection' : 'all' }));
  add('txted.dup', 'Edit: Duplicate Line', [M.CtrlCmd | M.Shift | K.KeyD], () => editor.trigger('kb', 'editor.action.copyLinesDownAction', null));
  add('txted.lineAfter', 'Edit: Insert Line After', [M.CtrlCmd | K.Enter], () => editor.trigger('kb', 'editor.action.insertLineAfter', null));
  add('txted.lineBefore', 'Edit: Insert Line Before', [M.CtrlCmd | M.Shift | K.Enter], () => editor.trigger('kb', 'editor.action.insertLineBefore', null));
  add('txted.splitLines', 'Selection: Split into Lines', [M.CtrlCmd | M.Shift | K.KeyL], () => editor.trigger('kb', 'editor.action.insertCursorAtEndOfEachLineSelected', null));
  add('txted.sortAsc', 'Edit: Sort Lines Ascending', [], () => editor.trigger('kb', 'editor.action.sortLinesAscending', null));
  add('txted.eol', 'Edit: Toggle Line Endings (LF/CRLF)', [], () => {
    if (!active) return;
    active.model.setEOL(active.model.getEOL() === '\n' ? monaco.editor.EndOfLineSequence.CRLF : monaco.editor.EndOfLineSequence.LF);
    updateStatus();
  });
  // Ctrl+Click adds cursors (Sublime style); Ctrl+D / Ctrl+K,Ctrl+D etc. are Monaco defaults.
}

function stepTab(d) {
  if (tabs.length < 2) return;
  activateTab(tabs[(tabs.indexOf(active) + d + tabs.length) % tabs.length]);
}

// ---------- compare / diff ----------
let diffEd = null, diffSt = null;

async function pickOneFile() {
  if (HAS_FS) {
    try { const [h] = await showOpenFilePicker(); return { name: h.name, text: await readFile(h) }; }
    catch (e) { if (e.name !== 'AbortError') toast(e.message); return null; }
  }
  return new Promise(res => {
    const i = document.createElement('input'); i.type = 'file';
    i.onchange = async () => res(i.files[0] ? { name: i.files[0].name, text: await i.files[0].text() } : null);
    i.click();
  });
}

function openDiff(left, right) {
  closeDiff();
  if (active) active.viewState = editor.saveViewState();
  if (!diffEd) {
    diffEd = monaco.editor.createDiffEditor($('diff'), {
      theme: 'monokai', automaticLayout: true, renderSideBySide: true, originalEditable: true,
      ignoreTrimWhitespace: false, fontSize: 14, fixedOverflowWidgets: true, useInlineViewWhenSpaceIsLimited: true,
      fontFamily: 'ui-monospace, Menlo, Consolas, "DejaVu Sans Mono", monospace',
    });
    diffEd.onDidUpdateDiff(updateDiffCount);
  }
  diffSt = { left, right, ws: false, side: true };
  diffEd.updateOptions({ renderSideBySide: true, ignoreTrimWhitespace: false });
  diffEd.setModel({ original: left.model, modified: right.model });
  $('diff-title').textContent = `${left.label}  ⇄  ${right.label}`;
  $('d-mode').textContent = 'Inline'; $('d-ws').textContent = 'Ignore whitespace: off';
  $('editor').hidden = true; $('diffwrap').hidden = false;
  updateDiffCount();
  diffEd.getModifiedEditor().focus();
}

function closeDiff() {
  if (!diffSt) return;
  const { left, right } = diffSt;
  diffSt = null;
  diffEd.setModel(null);
  for (const x of [left, right]) if (x.owned) x.model.dispose();
  $('diffwrap').hidden = true; $('editor').hidden = false;
  editor.layout(); editor.focus();
}

function updateDiffCount() {
  if (!diffSt) return;
  const c = diffEd.getLineChanges();
  $('diff-count').textContent = c === null ? 'comparing…' : c.length ? `${c.length} difference${c.length > 1 ? 's' : ''}` : 'no differences';
}

function stepDiff(dir) {
  const c = diffEd?.getLineChanges();
  if (!diffSt || !c?.length) return;
  const ed = diffEd.getModifiedEditor(), cur = ed.getPosition().lineNumber;
  const line = x => Math.max(1, x.modifiedStartLineNumber || x.modifiedEndLineNumber + 1);
  const target = dir > 0 ? c.find(x => line(x) > cur) || c[0] : [...c].reverse().find(x => line(x) < cur) || c[c.length - 1];
  ed.revealLineInCenter(line(target)); ed.setPosition({ lineNumber: line(target), column: 1 });
}

// ---------- format / minify ----------
let toastT = 0;
function toast(msg) {
  const el = $('st-sel'); el.textContent = msg; clearTimeout(toastT);
  toastT = setTimeout(updateStatus, 4000);
}

// Replace the selection (or the whole document if nothing is selected) with `text`.
function replaceTarget(range, text) {
  editor.executeEdits('txted-transform', [{ range, text, forceMoveMarkers: true }]);
  editor.pushUndoStop();
}

function targetRange() {
  const sel = editor.getSelection();
  return sel.isEmpty() ? active.model.getFullModelRange() : sel;
}

async function formatText() {
  if (!active) return;
  const lang = active.model.getLanguageId(), own = Transform.FORMATTERS[lang];
  try {
    if (own) {
      const r = targetRange();
      replaceTarget(r, await own(active.model.getValueInRange(r)));
    } else {
      const sel = editor.getSelection();
      const action = editor.getAction(sel.isEmpty() ? 'editor.action.formatDocument' : 'editor.action.formatSelection');
      if (!action?.isSupported()) return toast(`No formatter for ${lang}`);
      await action.run();
    }
    toast('Formatted');
  } catch (e) { toast('Format failed: ' + e.message); }
}

// Apply a line operation to the selected lines, or the whole document if nothing is selected.
function lineOp(name) {
  if (!active) return;
  const m = active.model, sel = editor.getSelection();
  let range = m.getFullModelRange();
  if (!sel.isEmpty()) {
    const endLine = sel.endColumn === 1 && sel.endLineNumber > sel.startLineNumber ? sel.endLineNumber - 1 : sel.endLineNumber;
    range = new monaco.Range(sel.startLineNumber, 1, endLine, m.getLineMaxColumn(endLine));
  }
  const eol = m.getEOL(), text = m.getValueInRange(range);
  const lines = text.split(/\r?\n/);
  const trailing = sel.isEmpty() && lines.length > 1 && lines[lines.length - 1] === '' ? lines.pop() !== undefined : false;
  const out = Transform.LINE_OPS[name](lines);
  replaceTarget(range, out.join(eol) + (trailing ? eol : ''));
  const removed = lines.length - out.length;
  toast(removed > 0 ? `${name}: removed ${removed} line${removed > 1 ? 's' : ''}` : `${name}: done`);
}

// Convert the selection (or whole document) into a new tab; the source tab is left alone.
function convert(label, outLang, fn) {
  if (!active) return;
  const r = targetRange(), src = active.model.getValueInRange(r);
  const isJava = label.startsWith('Java');
  const stem = active.name.replace(/\.[^.]*$/, '');
  const guess = /^untitled/i.test(stem) ? 'Root' : stem.replace(/[^A-Za-z0-9]+(.)?/g, (_, c) => (c || '').toUpperCase()).replace(/^./, c => c.toUpperCase());
  let root = guess;
  if (!isJava) {
    root = prompt('Name of the root Java class:', guess);
    if (!root) return;
  }
  try {
    const out = fn(src, root);
    const name = isJava ? stem + '.' + outLang : root.replace(/[^A-Za-z0-9_]/g, '') + '.java';
    addTab({ name, text: out, lang: isJava ? outLang : 'java' });
    toast('Converted: ' + label);
  } catch (e) { toast('Convert failed: ' + (e.message || e)); }
}

async function minifyText() {
  if (!active) return;
  const lang = active.model.getLanguageId(), fn = Transform.MINIFIERS[lang === 'typescript' ? 'javascript' : lang];
  if (!fn) return toast(`No minifier for ${lang}`);
  const r = targetRange(), src = active.model.getValueInRange(r);
  try {
    const out = await fn(src);
    replaceTarget(r, out);
    toast(`Minified: ${src.length} → ${out.length} chars`);
  } catch (e) { toast('Minify failed: ' + (e.message || e)); }
}

// ---------- status bar ----------
function updateStatus() {
  if (!active) { for (const id of ['st-pos', 'st-sel', 'st-indent', 'st-eol', 'st-lang']) $(id).textContent = ''; return; }
  const p = editor.getPosition(), sels = editor.getSelections() || [];
  $('st-pos').textContent = `Ln ${p.lineNumber}, Col ${p.column}`;
  const chars = sels.reduce((n, s) => n + active.model.getValueInRange(s).length, 0);
  $('st-sel').textContent = sels.length > 1 ? `${sels.length} selections` : chars ? `${chars} chars selected` : '';
  const o = active.model.getOptions();
  $('st-indent').textContent = (o.insertSpaces ? 'Spaces: ' : 'Tab Size: ') + o.tabSize;
  $('st-eol').textContent = active.model.getEOL() === '\n' ? 'LF' : 'CRLF';
  $('st-lang').textContent = active.model.getLanguageId();
}

function wireUI() {
  editor.onDidChangeCursorSelection(updateStatus);
  editor.onDidChangeModelContent(updateStatus);
  editor.onDidChangeModelLanguage?.(updateStatus);
  $('open-folder').onclick = openFolder;
  $('d-close').onclick = closeDiff;
  $('d-next').onclick = () => stepDiff(1);
  $('d-prev').onclick = () => stepDiff(-1);
  $('d-mode').onclick = () => {
    diffSt.side = !diffSt.side;
    diffEd.updateOptions({ renderSideBySide: diffSt.side, useInlineViewWhenSpaceIsLimited: false });
    $('d-mode').textContent = diffSt.side ? 'Inline' : 'Side by side';
  };
  $('d-ws').onclick = () => {
    diffSt.ws = !diffSt.ws;
    diffEd.updateOptions({ ignoreTrimWhitespace: diffSt.ws });
    $('d-ws').textContent = 'Ignore whitespace: ' + (diffSt.ws ? 'on' : 'off');
  };
  $('st-indent').onclick = () => {
    if (!active) return;
    const o = active.model.getOptions();
    active.model.updateOptions({ insertSpaces: !o.insertSpaces });
    updateStatus();
  };
  $('st-lang').onclick = setSyntax;
  // sidebar resize
  $('splitter').onpointerdown = e => {
    $('splitter').setPointerCapture(e.pointerId);
    $('splitter').onpointermove = ev => { $('sidebar').style.width = Math.max(120, Math.min(600, ev.clientX)) + 'px'; };
    $('splitter').onpointerup = () => { $('splitter').onpointermove = $('splitter').onpointerup = null; };
  };
  // drag & drop files
  addEventListener('dragover', e => e.preventDefault());
  addEventListener('drop', async e => {
    e.preventDefault();
    for (const it of e.dataTransfer.items) {
      if (it.kind !== 'file') continue;
      const h = HAS_FS && it.getAsFileSystemHandle ? await it.getAsFileSystemHandle() : null;
      if (h?.kind === 'file') await openHandle(h);
      else if (h?.kind === 'directory') await setFolder(h);
      else { const f = it.getAsFile(); if (f) addTab({ name: f.name, text: await f.text() }); }
    }
  });
  // files opened from the OS via the installed PWA
  if ('launchQueue' in window) launchQueue.setConsumer(async p => { for (const h of p.files) await openHandle(h); });
  // shortcuts that the editor can't see when focus is in the sidebar
  addEventListener('keydown', e => {
    if ((e.ctrlKey || e.metaKey) && !e.shiftKey && e.key === 's') {
      e.preventDefault();
      if (diffSt) for (const x of [diffSt.right, diffSt.left]) { const t = tabs.find(t => t.model === x.model); if (t && isDirty(t)) saveTab(t); }
    }
    if (diffSt && e.key === 'F7') { e.preventDefault(); stepDiff(e.shiftKey ? -1 : 1); }
    if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === 'p') e.preventDefault();
  });
  addEventListener('beforeunload', e => { if (tabs.some(isDirty) && !HAS_FS) e.preventDefault(); flushSave(); });
}

// ---------- session persistence ----------
const MAX_STORED = 2e6;   // chars; bigger buffers aren't copied into the session store
let saveTimer = 0, pending = false;
function scheduleSave() { pending = true; clearTimeout(saveTimer); saveTimer = setTimeout(flushSave, 1500); }
function flushSave() {
  clearTimeout(saveTimer);
  if (!pending) return;
  pending = false;
  const s = {
    active: tabs.indexOf(active), untitledN,
    tabs: tabs.map(t => {
      const dirty = isDirty(t), len = t.model.getValueLength();
      // clean file-backed tabs are re-read from disk on restore, so don't store their text
      const keep = (dirty || !t.handle) && len <= MAX_STORED;
      return { name: t.name, handle: t.handle, text: keep ? t.model.getValue() : null, dirty, lang: t.model.getLanguageId() };
    }),
  };
  return db.set('session', s);
}

async function restoreSession() {
  // ?reset wipes saved state; a previous boot that never finished is treated as a crash and skipped.
  let skip = false;
  try {
    if (/[?&]reset\b/.test(location.search)) {
      await db.set('session', null); await db.set('folder', null);
      history.replaceState(null, '', location.pathname);
    } else if (localStorage.getItem('txted-booting')) {
      skip = true;
      await db.set('session-backup', await db.get('session'));
      await db.set('session', null);
    }
    localStorage.setItem('txted-booting', '1');
  } catch { /* storage blocked: carry on */ }

  const s = skip ? null : await db.get('session');
  if (s?.tabs?.length) {
    untitledN = s.untitledN || 0;
    for (const x of s.tabs) {
      let text = x.text;
      if (x.handle && !x.dirty && await x.handle.queryPermission?.({ mode: 'read' }) === 'granted') {
        try { text = await readFile(x.handle); } catch { /* fall through */ }
      }
      if (text == null) continue;   // nothing to show (large/clean file we can't re-read yet)
      addTab({ name: x.name, handle: x.handle, text, dirty: x.dirty, lang: x.lang, activate: false });
    }
    if (tabs.length) activateTab(tabs[Math.max(0, Math.min(s.active, tabs.length - 1))]);
  }
  if (!tabs.length) newTab();
  if (skip) toast('Previous session failed to load and was skipped (backed up)');
  try { localStorage.removeItem('txted-booting'); } catch { }

  const fh = await db.get('folder');
  if (fh) {
    $('folder-name').textContent = fh.name;
    const b = document.createElement('button');
    b.className = 'restore'; b.textContent = `Reopen "${fh.name}"`;
    b.onclick = async () => {
      if (await fh.requestPermission({ mode: 'readwrite' }) === 'granted') await setFolder(fh);
    };
    $('tree').replaceChildren(b);
  }
}
