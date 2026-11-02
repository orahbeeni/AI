// ==UserScript==
// @name         mpern-env-ribbon Top Bar + Toolbar
// @namespace    http://tampermonkey.net/
// @version      2.30
// @description  Moves mpern-env-ribbon to a top-center bar at 40% opacity, adds a quick-nav toolbar and a notes panel
// @match        *://*/*backoffice/*
// @match        https://localhost:4200/*
// @match        https://*.scec.co.za/*
// @match        *://*/hac/login*
// @match        *://*/authorizationserver/login*
// @exclude-match https://portal.commerce.ondemand.com/*
// @noframes
// @grant        GM_getValue
// @grant        GM_setValue
// @grant        unsafeWindow
// @run-at       document-start
// ==/UserScript==
(function () {
    'use strict';

    /* ------------------------------------------------------------------ */
    /* Ribbon styling (unchanged from v2.0)                                */
    /* ------------------------------------------------------------------ */
    const style = document.createElement('style');
    style.textContent = `
        .mpern-env-ribbon {
            position: fixed !important;
            top: 0 !important;
            left: 90mm !important;
            transform: none !important;
            right: auto !important;
            bottom: auto !important;
            width: auto !important;
            padding: 4px 20px !important;
            border-radius: 0 0 6px 6px !important;
            text-align: center !important;
            opacity: 0.4 !important;
            z-index: 2147483647 !important;
            pointer-events: none !important;
        }

        .mpern-toolbar {
            position: fixed !important;
            top: 6px !important;
            left: 50% !important;
            transform: translateX(-50%) !important;
            right: auto !important;
            display: flex !important;
            gap: 6px !important;
            z-index: 2147483647 !important;
            font-family: -apple-system, "Segoe UI", Roboto, Arial, sans-serif !important;
        }

        .mpern-toolbar-btn {
            background: rgba(90, 90, 90, 0.85) !important;
            color: #fff !important;
            border: 1px solid rgba(255, 255, 255, 0.15) !important;
            padding: 4px 12px !important;
            font-size: 12px !important;
            font-weight: 600 !important;
            letter-spacing: 0.02em !important;
            border-radius: 6px !important;
            cursor: pointer !important;
            box-shadow: 0 2px 6px rgba(0, 0, 0, 0.25) !important;
            transition: filter 0.15s ease, transform 0.1s ease !important;
        }

        /* Colour groups */
        .mpern-toolbar-btn--grey {
            background: rgba(190, 190, 190, 0.9) !important;
            color: #1a1a1a !important;
        }
        .mpern-toolbar-btn--black {
            background: rgba(15, 15, 15, 0.92) !important;
            color: #fff !important;
        }
        .mpern-toolbar-btn--blue {
            background: rgba(36, 150, 237, 0.92) !important; /* Docker blue */
            color: #fff !important;
        }
        .mpern-toolbar-btn--green {
            background: rgba(39, 142, 92, 0.92) !important;
            color: #fff !important;
        }
        .mpern-toolbar-btn--orange {
            background: rgba(224, 130, 40, 0.92) !important;
            color: #fff !important;
        }
        .mpern-toolbar-btn--red {
            background: rgba(197, 55, 48, 0.92) !important;
            color: #fff !important;
        }

        .mpern-toolbar-btn:hover {
            filter: brightness(1.15) !important;
            transform: translateY(-1px) !important;
        }

        .mpern-toolbar-btn:active {
            transform: translateY(0) !important;
        }

        .mpern-toolbar-btn.is-busy {
            opacity: 0.6 !important;
            cursor: wait !important;
        }

        .mpern-toolbar-btn.is-error {
            background: rgba(180, 40, 40, 0.9) !important;
        }

        /* "..." button with a sub-menu that drops down on hover */
        .mpern-toolbar-more {
            position: relative !important;
        }
        .mpern-toolbar-submenu {
            display: none !important;
            position: absolute !important;
            top: 100% !important;
            right: 0 !important;
            padding-top: 4px !important; /* gap that still counts as hovering */
            flex-direction: column !important;
            gap: 4px !important;
            white-space: nowrap !important;
        }
        .mpern-toolbar-more:hover .mpern-toolbar-submenu {
            display: flex !important;
        }

        /* Notes panel, docked on the right */
        .mpern-notes {
            position: fixed !important;
            top: 40px !important;
            right: 0 !important;
            display: flex !important;
            flex-direction: column !important;
            box-sizing: border-box !important;
            --mn-bg: rgba(30, 30, 30, 0.96);
            --mn-fg: #eee;
            --mn-border: rgba(255, 255, 255, 0.15);
            --mn-note: rgba(255, 255, 255, 0.06);
            background: var(--mn-bg) !important;
            color: var(--mn-fg) !important;
            border: 1px solid var(--mn-border) !important;
            border-right: none !important;
            border-radius: 6px 0 0 6px !important;
            box-shadow: 0 4px 16px rgba(0, 0, 0, 0.4) !important;
            z-index: 2147483646 !important;
            font-family: -apple-system, "Segoe UI", Roboto, Arial, sans-serif !important;
            font-size: 12px !important;
        }
        .mpern-notes.is-day {
            --mn-bg: rgba(250, 250, 247, 0.97);
            --mn-fg: #1f1f1f;
            --mn-border: rgba(0, 0, 0, 0.2);
            --mn-note: rgba(0, 0, 0, 0.05);
        }
        .mpern-notes.is-hidden {
            display: none !important;
        }
        .mpern-notes-header {
            display: flex !important;
            align-items: center !important;
            gap: 4px !important;
            padding: 6px !important;
            border-bottom: 1px solid var(--mn-border) !important;
            cursor: move !important;
            user-select: none !important;
        }
        .mpern-notes-header .mpern-toolbar-btn {
            padding: 3px 8px !important;
        }
        .mpern-notes-header .mpern-toolbar-btn.is-active {
            background: rgba(36, 150, 237, 0.92) !important;
        }
        .mpern-notes-spacer {
            flex: 1 !important;
        }
        .mpern-notes-list {
            flex: 1 !important;
            overflow-y: auto !important;
            padding: 6px !important;
            display: flex !important;
            flex-direction: column !important;
            gap: 6px !important;
        }
        .mpern-notes-empty {
            opacity: 0.5 !important;
            text-align: center !important;
            padding: 12px !important;
        }
        .mpern-note {
            background: var(--mn-note) !important;
            border-radius: 4px !important;
            padding: 4px !important;
        }
        .mpern-note-editor {
            min-height: 60px !important;
            outline: none !important;
            white-space: pre-wrap !important;
            overflow-wrap: anywhere !important;
            font: 12px/1.4 Menlo, Consolas, monospace !important;
        }
        /* Wrap off: long lines stay on one line and each note scrolls sideways */
        .mpern-notes.is-nowrap .mpern-note-editor {
            white-space: pre !important;
            overflow-wrap: normal !important;
            overflow-x: auto !important;
        }
        .mpern-notes-format .mpern-toolbar-btn.is-active {
            background: rgba(36, 150, 237, 0.92) !important;
        }
        .mpern-notes-format {
            display: flex !important;
            align-items: center !important;
            flex-wrap: wrap !important;
            gap: 4px !important;
            padding: 4px 6px !important;
            border-bottom: 1px solid var(--mn-border) !important;
        }
        .mpern-notes-format .mpern-toolbar-btn {
            padding: 1px 7px !important;
            min-width: 22px !important;
        }
        .mpern-notes-swatch {
            width: 16px !important;
            height: 16px !important;
            padding: 0 !important;
            border: 1px solid var(--mn-border) !important;
            border-radius: 50% !important;
            cursor: pointer !important;
        }
        .mpern-note-footer {
            display: flex !important;
            justify-content: space-between !important;
            align-items: center !important;
            opacity: 0.55 !important;
            font-size: 10px !important;
        }
        .mpern-note-delete {
            background: none !important;
            border: none !important;
            color: inherit !important;
            cursor: pointer !important;
            font-size: 12px !important;
        }
        .mpern-note-delete:hover {
            color: #f66 !important;
        }
        .mpern-notes-resize-w,
        .mpern-notes-resize-s,
        .mpern-notes-resize-sw {
            position: absolute !important;
        }
        .mpern-notes-resize-w {
            top: 0 !important;
            bottom: 0 !important;
            left: -3px !important;
            width: 6px !important;
            cursor: ew-resize !important;
        }
        .mpern-notes-resize-s {
            left: 0 !important;
            right: 0 !important;
            bottom: -3px !important;
            height: 6px !important;
            cursor: ns-resize !important;
        }
        .mpern-notes-resize-sw {
            left: -4px !important;
            bottom: -4px !important;
            width: 10px !important;
            height: 10px !important;
            cursor: nesw-resize !important;
        }
    `;
    document.documentElement.appendChild(style);

    /* ------------------------------------------------------------------ */
    /* Helpers                                                             */
    /* ------------------------------------------------------------------ */
    const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));

    // With GM_* grants the script runs sandboxed, so `window` may be a
    // wrapper that MouseEvent won't accept as its view.
    const pageWindow = typeof unsafeWindow !== 'undefined' ? unsafeWindow : window;

    // Finds the explorer tree's filter input by placeholder text (not id,
    // since ZK ids are regenerated per session).
    function findTreeFilterInput() {
        const candidates = Array.from(
            document.querySelectorAll('input[placeholder^="Filter tree"]')
        );
        // Prefer a visible one, in case more than one tree/filter exists on screen.
        return candidates.find((el) => el.offsetParent !== null) || candidates[0] || null;
    }

    // Sets an <input> value using the native setter so any framework
    // listening via property descriptors still sees the change.
    function setNativeValue(el, value) {
        const proto = Object.getPrototypeOf(el);
        const setter = Object.getOwnPropertyDescriptor(proto, 'value') && Object.getOwnPropertyDescriptor(proto, 'value').set;
        if (setter) {
            setter.call(el, value);
        } else {
            el.value = value;
        }
    }

    function fireKeyEvent(el, type, key) {
        el.dispatchEvent(new KeyboardEvent(type, { key, bubbles: true, cancelable: true }));
    }

    // Simulates real character-by-character typing (not just setting .value),
    // since the tree filter listens for keyup/input to trigger its (debounced)
    // AJAX filter request.
    async function typeIntoFilter(input, text, perCharDelayMs = 45) {
        input.focus();
        setNativeValue(input, '');
        input.dispatchEvent(new Event('input', { bubbles: true }));

        let current = '';
        for (const char of text) {
            current += char;
            fireKeyEvent(input, 'keydown', char);
            fireKeyEvent(input, 'keypress', char);
            setNativeValue(input, current);
            input.dispatchEvent(new Event('input', { bubbles: true }));
            fireKeyEvent(input, 'keyup', char);
            await sleep(perCharDelayMs);
        }
        input.dispatchEvent(new Event('change', { bubbles: true }));
    }

    // Polls for a visible tree label with exact matching text, since the
    // filtered result renders asynchronously after the AJAX round-trip.
    //
    // The filter can match multiple nodes with identical text (e.g. a group
    // node and the actual type node nested under it). When iconMatch is
    // given, only a candidate whose tree-cell contains an icon element with
    // a class matching that substring is accepted — this is how we tell the
    // real "Business Processes" type node apart from its parent group.
    //
    // On a first-time-after-login tree state, the parent group can render
    // collapsed, with the real (iconMatch) node hidden underneath it and
    // therefore not yet visible (offsetParent === null). In that case we
    // click the visible parent once to expand it, then keep polling for the
    // real node to become visible.
    async function waitForTreeLabel(text, { timeoutMs = 6000, intervalMs = 200, iconMatch = null } = {}) {
        const deadline = Date.now() + timeoutMs;
        const isVisible = (el) => el.offsetParent !== null;
        const hasIcon = (el) => {
            const cell = el.closest('.yw-treeCellInner');
            return !!(cell && cell.querySelector(`[class*="${iconMatch}"]`));
        };

        let expandedParent = false;

        while (Date.now() < deadline) {
            const candidates = Array.from(document.querySelectorAll('span.z-label')).filter(
                (el) => el.textContent.trim() === text
            );

            if (iconMatch) {
                const match = candidates.find((el) => isVisible(el) && hasIcon(el));
                if (match) return match;

                if (!expandedParent) {
                    const parent = candidates.find((el) => isVisible(el) && !hasIcon(el));
                    if (parent) {
                        simulateClick(parent.closest('.yw-treeCellInner') || parent);
                        expandedParent = true; // only try expanding once
                    }
                }
            } else {
                const match = candidates.find(isVisible);
                if (match) return match;
            }

            await sleep(intervalMs);
        }
        return null;
    }

    function simulateClick(el) {
        ['mousedown', 'mouseup', 'click'].forEach((type) => {
            el.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, view: pageWindow }));
        });
    }

    // Types into the tree filter, waits for the matching node to appear,
    // then clicks it (clicking the whole tree-cell, not just the label,
    // since that's usually the row's actual click target).
    //
    // nodeText is what we search for and click on. filterText — if given —
    // is what actually gets typed into the box, for cases where a shorter
    // query still narrows the tree down to the target node.
    async function filterAndOpenTreeNode(nodeText, { iconMatch = null, filterText = null } = {}) {
        const input = findTreeFilterInput();
        if (!input) {
            console.warn('[mpern-toolbar] Could not find the tree filter input.');
            return false;
        }

        await typeIntoFilter(input, filterText || nodeText);

        const label = await waitForTreeLabel(nodeText, { iconMatch });
        if (!label) {
            console.warn(`[mpern-toolbar] "${nodeText}" did not appear in the tree after filtering.`);
            return false;
        }

        const target = label.closest('.yw-treeCellInner') || label;
        simulateClick(target);
        return true;
    }

    /* ------------------------------------------------------------------ */
    /* Toolbar                                                             */
    /* ------------------------------------------------------------------ */

    // Add new buttons here as needed — each just needs a label and an
    // async onClick. Busy/error states are handled generically below.
    const TOOLBAR_BUTTONS = [
        {
            label: 'BPs',
            color: 'grey',
            onClick: () =>
                filterAndOpenTreeNode('Business Processes', {
                    iconMatch: 'hmc_typenode_businessprocess',
                }),
        },
        { label: 'CJs', color: 'grey', onClick: () => filterAndOpenTreeNode('CronJobs') },
        { label: 'Maps', color: 'black', onClick: () => filterAndOpenTreeNode('Context Value Mapping') },
        { label: 'IMRs', color: 'black', onClick: () => filterAndOpenTreeNode('Integration Message record') },
        { label: 'Products', color: 'blue', onClick: () => filterAndOpenTreeNode('Products') },
        { label: 'Orders', color: 'blue', onClick: () => filterAndOpenTreeNode('Orders') },
        { label: 'Carts', color: 'blue', onClick: () => filterAndOpenTreeNode('Carts') },
        { label: 'Cust', color: 'green', onClick: () => filterAndOpenTreeNode('Customers') },
        { label: 'Empl', color: 'green', onClick: () => filterAndOpenTreeNode('Employees') },
        { label: 'Facet', color: 'orange', onClick: () => filterAndOpenTreeNode('Facet Search Configurations') },
        { label: 'Patch', color: 'orange', onClick: () => filterAndOpenTreeNode('Patch Execution') },
        { label: 'B2B', color: 'red', onClick: () => filterAndOpenTreeNode('B2B Unit', { filterText: 'B2B' }) },
        { label: 'Types', onClick: () => filterAndOpenTreeNode('Types') },
        { label: 'Media', onClick: () => filterAndOpenTreeNode('Media') },
        { label: 'Script', onClick: () => filterAndOpenTreeNode('Scripting') },
        {
            label: '...',
            children: [
                { label: 'PoS', onClick: () => filterAndOpenTreeNode('Point of Service') },
                { label: 'Website', onClick: () => filterAndOpenTreeNode('Website') },
                { label: 'BaseStore', onClick: () => filterAndOpenTreeNode('Base Store') },
            ],
        },
        {
            label: 'Notes',
            onClick: () => {
                toggleNotesPanel();
                return true;
            },
        },
    ];

    function buildToolbar() {
        if (document.querySelector('.mpern-toolbar')) return; // avoid duplicates

        const bar = document.createElement('div');
        bar.className = 'mpern-toolbar';

        TOOLBAR_BUTTONS.forEach((def) => {
            if (!def.children) {
                bar.appendChild(buildButton(def));
                return;
            }

            // Buttons with children show them in a sub-menu below on hover.
            const wrap = document.createElement('div');
            wrap.className = 'mpern-toolbar-more';
            wrap.appendChild(buildButton(def));

            const submenu = document.createElement('div');
            submenu.className = 'mpern-toolbar-submenu';
            def.children.forEach((child) => submenu.appendChild(buildButton(child)));
            wrap.appendChild(submenu);

            bar.appendChild(wrap);
        });

        document.body.appendChild(bar);
    }

    function buildButton({ label, color, onClick }) {
        const btn = document.createElement('button');
        btn.type = 'button';
        btn.className = color ? `mpern-toolbar-btn mpern-toolbar-btn--${color}` : 'mpern-toolbar-btn';
        btn.textContent = label;

        if (onClick) {
            btn.addEventListener('click', async () => {
                if (btn.classList.contains('is-busy')) return;
                btn.classList.remove('is-error');
                btn.classList.add('is-busy');
                try {
                    const ok = await onClick();
                    if (!ok) {
                        btn.classList.add('is-error');
                        setTimeout(() => btn.classList.remove('is-error'), 1500);
                    }
                } catch (err) {
                    console.error('[mpern-toolbar] Button action failed:', err);
                    btn.classList.add('is-error');
                    setTimeout(() => btn.classList.remove('is-error'), 1500);
                } finally {
                    btn.classList.remove('is-busy');
                }
            });
        }

        return btn;
    }

    /* ------------------------------------------------------------------ */
    /* Notes panel                                                         */
    /* ------------------------------------------------------------------ */

    // Everything is kept in Tampermonkey's own storage (GM_getValue /
    // GM_setValue), not the site's, so it survives clearing cookies and
    // site data, and the same notes show up on every host.
    //
    // Keys:
    //   notes.<category>  array of { id, html, text, created, updated }, newest first
    //                     (older notes only have `text`; it is converted on first render)
    //   notes.theme       'night' (default) or 'day'
    //   notes.wrap        whether long lines wrap (default true) or scroll sideways
    //   notes.sizes       { <category>: { width, height } }
    //   notes.pos         { left, top } once the panel has been dragged;
    //                     absent while it is still docked to the right edge
    //   notes.open        whether the panel was last left open
    //   notes.category    the last category picked by hand
    const NOTE_CATEGORIES = ['Dev', 'INT(D1)', 'QA(S1)', 'PROD'];
    const NOTES_DEFAULT_SIZE = { width: 360, height: 480 };
    const NOTES_MIN_SIZE = { width: 220, height: 160 };

    let notesPanel = null;
    let notesList = null;
    let notesCategory = guessNoteCategory() || GM_getValue('notes.category', NOTE_CATEGORIES[0]);

    // Picks the category from the host name where it's obvious
    // (localhost, or a CCv2 host with -d1 / -s1 / -p1 in it).
    function guessNoteCategory() {
        const host = location.hostname;
        if (host === 'localhost' || host === '127.0.0.1') return 'Dev';
        if (/(^|[.-])d1([.-]|$)/i.test(host)) return 'INT(D1)';
        if (/(^|[.-])s1([.-]|$)/i.test(host)) return 'QA(S1)';
        if (/(^|[.-])p1([.-]|$)/i.test(host)) return 'PROD';
        return null;
    }

    function loadNotes(category) {
        return GM_getValue(`notes.${category}`, []);
    }

    function saveNotes(category, notes) {
        GM_setValue(`notes.${category}`, notes);
    }

    const NOTE_COLORS = ['#e53935', '#fb8c00', '#f9a825', '#43a047', '#00acc1', '#1e88e5', '#8e24aa', '#ec407a'];

    const escapeHtml = (str) =>
        str.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');

    // Re-reads before writing, so a note edited in another tab isn't
    // overwritten by this tab's stale copy of the list.
    function updateNoteContent(category, id, html, text) {
        const notes = loadNotes(category);
        const note = notes.find((n) => n.id === id);
        if (!note) return;
        note.html = html;
        note.text = text;
        note.updated = Date.now();
        saveNotes(category, notes);
    }

    function loadNotesSize(category) {
        const sizes = GM_getValue('notes.sizes', {});
        return sizes[category] || NOTES_DEFAULT_SIZE;
    }

    function saveNotesSize(category, size) {
        const sizes = GM_getValue('notes.sizes', {});
        sizes[category] = size;
        GM_setValue('notes.sizes', sizes);
    }

    function applyNotesSize({ width, height }) {
        const maxWidth = window.innerWidth - 20;
        const maxHeight = window.innerHeight - notesPanel.offsetTop - 4;
        notesPanel.style.width = `${Math.max(NOTES_MIN_SIZE.width, Math.min(width, maxWidth))}px`;
        notesPanel.style.height = `${Math.max(NOTES_MIN_SIZE.height, Math.min(height, maxHeight))}px`;
        applyNotesPos(); // a different size may push a dragged panel off-screen
    }

    // Once dragged, the panel is placed with left/top instead of the CSS
    // default of "docked right". Clamped so the header always stays reachable.
    // The stylesheet uses !important, so these have to be set as important too.
    function applyNotesPos() {
        const pos = GM_getValue('notes.pos', null);
        if (!pos || notesPanel.classList.contains('is-hidden')) return null;

        const left = Math.max(0, Math.min(pos.left, window.innerWidth - notesPanel.offsetWidth));
        const top = Math.max(0, Math.min(pos.top, window.innerHeight - 40));
        notesPanel.style.setProperty('left', `${left}px`, 'important');
        notesPanel.style.setProperty('top', `${top}px`, 'important');
        notesPanel.style.setProperty('right', 'auto', 'important');
        notesPanel.style.setProperty('border-right', '1px solid var(--mn-border)', 'important');
        notesPanel.style.setProperty('border-radius', '6px', 'important');
        return { left, top };
    }

    function startNotesDrag(e) {
        if (e.button !== 0 || e.target.closest('button')) return;
        e.preventDefault();
        const startX = e.clientX;
        const startY = e.clientY;
        const start = notesPanel.getBoundingClientRect();
        let moved = false;

        const onMove = (ev) => {
            moved = true;
            GM_setValue('notes.pos', { left: start.left + ev.clientX - startX, top: start.top + ev.clientY - startY });
            applyNotesPos();
        };
        const onUp = () => {
            document.removeEventListener('mousemove', onMove);
            document.removeEventListener('mouseup', onUp);
            document.body.style.userSelect = '';
            // Store the clamped position, not the raw mouse-derived one.
            if (moved) GM_setValue('notes.pos', applyNotesPos());
        };

        document.body.style.userSelect = 'none';
        document.addEventListener('mousemove', onMove);
        document.addEventListener('mouseup', onUp);
    }

    function buildNotesPanel() {
        notesPanel = document.createElement('div');
        notesPanel.className = 'mpern-notes is-hidden';

        // Keep typing in a note from reaching the backoffice's own key handlers.
        ['keydown', 'keypress', 'keyup'].forEach((type) =>
            notesPanel.addEventListener(type, (e) => e.stopPropagation())
        );

        const header = document.createElement('div');
        header.className = 'mpern-notes-header';

        NOTE_CATEGORIES.forEach((category) => {
            const tab = buildButton({
                label: category,
                onClick: () => {
                    GM_setValue('notes.category', category);
                    showNotesCategory(category);
                    return true;
                },
            });
            tab.dataset.category = category;
            header.appendChild(tab);
        });

        const spacer = document.createElement('div');
        spacer.className = 'mpern-notes-spacer';
        header.appendChild(spacer);

        const themeBtn = buildButton({
            label: '',
            onClick: () => {
                setNotesTheme(notesPanel.classList.contains('is-day') ? 'night' : 'day');
                return true;
            },
        });
        themeBtn.classList.add('mpern-notes-theme');
        header.appendChild(themeBtn);

        const addBtn = buildButton({ label: '+', color: 'green', onClick: () => addNote() || true });
        addBtn.title = 'New note';
        header.appendChild(addBtn);

        const minBtn = buildButton({ label: '–', onClick: () => toggleNotesPanel() || true });
        minBtn.title = 'Minimise notes';
        header.appendChild(minBtn);

        header.addEventListener('mousedown', startNotesDrag);

        notesList = document.createElement('div');
        notesList.className = 'mpern-notes-list';

        notesPanel.appendChild(header);
        notesPanel.appendChild(buildFormatBar());
        notesPanel.appendChild(notesList);
        setNotesTheme(GM_getValue('notes.theme', 'night'));
        setNotesWrap(GM_getValue('notes.wrap', true));

        [['w', 'mpern-notes-resize-w'], ['s', 'mpern-notes-resize-s'], ['sw', 'mpern-notes-resize-sw']].forEach(
            ([dir, className]) => {
                const handle = document.createElement('div');
                handle.className = className;
                handle.addEventListener('mousedown', (e) => startNotesResize(e, dir));
                notesPanel.appendChild(handle);
            }
        );

        document.body.appendChild(notesPanel);
        window.addEventListener('resize', applyNotesPos);

        if (GM_getValue('notes.open', false)) toggleNotesPanel();
    }

    function setNotesTheme(theme) {
        const day = theme === 'day';
        notesPanel.classList.toggle('is-day', day);
        const btn = notesPanel.querySelector('.mpern-notes-theme');
        btn.textContent = day ? '☀' : '☾';
        btn.title = day ? 'Day mode (click for night)' : 'Night mode (click for day)';
        GM_setValue('notes.theme', day ? 'day' : 'night');
    }

    function setNotesWrap(wrap) {
        notesPanel.classList.toggle('is-nowrap', !wrap);
        const btn = notesPanel.querySelector('.mpern-notes-wrap');
        btn.classList.toggle('is-active', wrap);
        btn.title = wrap ? 'Wrapping long lines (click for horizontal scroll)' : 'Scrolling sideways (click to wrap)';
        GM_setValue('notes.wrap', wrap);
    }

    // Applies a formatting command to the selection inside whichever note is
    // being edited. The buttons swallow mousedown so the selection isn't lost.
    function formatNote(command, value = null) {
        const sel = window.getSelection();
        const anchor = sel && sel.anchorNode && (sel.anchorNode.nodeType === 1 ? sel.anchorNode : sel.anchorNode.parentElement);
        const editor = anchor && anchor.closest('.mpern-note-editor');
        if (!editor) return;
        document.execCommand('styleWithCSS', false, true);
        if (command === 'fontSize') {
            // execCommand only knows the 7 legacy keyword sizes, which aren't
            // relative to the editor's 12px text. Apply the largest as a
            // marker, then swap in the real pixel size.
            document.execCommand('fontSize', false, '7');
            editor.querySelectorAll('[style*="xxx-large"], font[size="7"]').forEach((el) => {
                el.removeAttribute('size');
                el.style.fontSize = value;
            });
        } else {
            document.execCommand(command, false, value);
        }
        editor.dispatchEvent(new Event('input', { bubbles: true }));
    }

    function buildFormatBar() {
        const bar = document.createElement('div');
        bar.className = 'mpern-notes-format';
        bar.addEventListener('mousedown', (e) => e.preventDefault());

        const add = (label, title, command, value, style) => {
            const btn = buildButton({ label, onClick: () => formatNote(command, value) || true });
            btn.title = title;
            if (style) btn.style.cssText += style;
            bar.appendChild(btn);
        };

        add('B', 'Bold', 'bold', null, 'font-weight:900 !important;');
        add('I', 'Italic', 'italic', null, 'font-style:italic !important;');
        add('U', 'Underline', 'underline', null, 'text-decoration:underline !important;');
        add('S', 'Small text', 'fontSize', '10px', 'font-size:10px !important;');
        add('M', 'Medium text', 'fontSize', '14px', 'font-size:13px !important;');
        add('L', 'Large text', 'fontSize', '18px', 'font-size:16px !important;');

        NOTE_COLORS.forEach((color) => {
            const swatch = document.createElement('button');
            swatch.type = 'button';
            swatch.className = 'mpern-notes-swatch';
            swatch.title = `Text colour ${color}`;
            swatch.style.setProperty('background', color, 'important');
            swatch.addEventListener('click', () => formatNote('foreColor', color));
            bar.appendChild(swatch);
        });

        add('T✕', 'Clear formatting', 'removeFormat');

        const wrapBtn = buildButton({
            label: 'Wrap',
            onClick: () => {
                setNotesWrap(notesPanel.classList.contains('is-nowrap'));
                return true;
            },
        });
        wrapBtn.classList.add('mpern-notes-wrap');
        bar.appendChild(wrapBtn);
        return bar;
    }

    function toggleNotesPanel() {
        if (!notesPanel) return;
        const opening = notesPanel.classList.contains('is-hidden');
        notesPanel.classList.toggle('is-hidden', !opening);
        GM_setValue('notes.open', opening);
        // Re-render on open to pick up anything written in another tab.
        if (opening) showNotesCategory(notesCategory);
    }

    function showNotesCategory(category) {
        notesCategory = category;
        notesPanel.querySelectorAll('[data-category]').forEach((tab) => {
            tab.classList.toggle('is-active', tab.dataset.category === category);
        });
        applyNotesSize(loadNotesSize(category));
        renderNotes();
    }

    function renderNotes() {
        const category = notesCategory;
        const notes = loadNotes(category);
        notesList.replaceChildren();

        if (!notes.length) {
            const empty = document.createElement('div');
            empty.className = 'mpern-notes-empty';
            empty.textContent = `No ${category} notes yet — press + to add one.`;
            notesList.appendChild(empty);
            return;
        }

        notes.forEach((note) => {
            const item = document.createElement('div');
            item.className = 'mpern-note';

            const editor = document.createElement('div');
            editor.className = 'mpern-note-editor';
            editor.contentEditable = 'true';
            editor.spellcheck = false;
            editor.innerHTML = note.html !== undefined ? note.html : escapeHtml(note.text || '');
            editor.addEventListener('input', () =>
                updateNoteContent(category, note.id, editor.innerHTML, editor.innerText)
            );
            // Paste as plain text so foreign styling doesn't leak into notes.
            editor.addEventListener('paste', (e) => {
                e.preventDefault();
                document.execCommand('insertText', false, e.clipboardData.getData('text/plain'));
            });

            const footer = document.createElement('div');
            footer.className = 'mpern-note-footer';

            const date = document.createElement('span');
            date.textContent = new Date(note.created).toLocaleString();

            const del = document.createElement('button');
            del.type = 'button';
            del.className = 'mpern-note-delete';
            del.textContent = '✕';
            del.title = 'Delete note';
            del.addEventListener('click', () => {
                if (!confirm('Delete this note?')) return;
                saveNotes(category, loadNotes(category).filter((n) => n.id !== note.id));
                renderNotes();
            });

            footer.appendChild(date);
            footer.appendChild(del);
            item.appendChild(editor);
            item.appendChild(footer);
            notesList.appendChild(item);
        });
    }

    function addNote() {
        const category = notesCategory;
        const now = Date.now();
        const note = { id: `${now}-${Math.random().toString(36).slice(2, 8)}`, html: '', text: '', created: now, updated: now };
        saveNotes(category, [note, ...loadNotes(category)]);
        renderNotes();
        const first = notesList.querySelector('.mpern-note-editor');
        if (first) first.focus();
    }

    // The panel is anchored to the right edge, so dragging the left edge
    // left makes it wider; dragging the bottom edge down makes it taller.
    function startNotesResize(e, dir) {
        e.preventDefault();
        const category = notesCategory;
        const startX = e.clientX;
        const startY = e.clientY;
        const start = notesPanel.getBoundingClientRect();
        let size = { width: start.width, height: start.height };

        const onMove = (ev) => {
            size = {
                width: dir.includes('w') ? start.width + startX - ev.clientX : start.width,
                height: dir.includes('s') ? start.height + ev.clientY - startY : start.height,
            };
            applyNotesSize(size);
            // Dragged panels are positioned by their left edge, so dragging
            // the left handle has to move it to keep the right edge in place.
            if (dir.includes('w') && GM_getValue('notes.pos', null)) {
                GM_setValue('notes.pos', { left: start.right - notesPanel.offsetWidth, top: start.top });
                applyNotesPos();
            }
        };
        const onUp = () => {
            document.removeEventListener('mousemove', onMove);
            document.removeEventListener('mouseup', onUp);
            document.body.style.userSelect = '';
            const rect = notesPanel.getBoundingClientRect();
            saveNotesSize(category, { width: Math.round(rect.width), height: Math.round(rect.height) });
        };

        document.body.style.userSelect = 'none';
        document.addEventListener('mousemove', onMove);
        document.addEventListener('mouseup', onUp);
    }

    function onBodyReady(callback) {
        if (document.body) {
            callback();
            return;
        }
        const observer = new MutationObserver(() => {
            if (document.body) {
                observer.disconnect();
                callback();
            }
        });
        observer.observe(document.documentElement, { childList: true });
    }

    /* ------------------------------------------------------------------ */
    /* Login page auto-fill                                                */
    /* ------------------------------------------------------------------ */

    // Generic "wait for a selector to exist" helper, for pages (like the
    // login form) where we're not searching the explorer tree.
    async function waitForElement(selector, timeoutMs = 5000, intervalMs = 150) {
        const deadline = Date.now() + timeoutMs;
        while (Date.now() < deadline) {
            const el = document.querySelector(selector);
            if (el) return el;
            await sleep(intervalMs);
        }
        return null;
    }

    function fillField(el, value) {
        setNativeValue(el, value);
        el.dispatchEvent(new Event('input', { bubbles: true }));
        el.dispatchEvent(new Event('change', { bubbles: true }));
    }

    async function fillLoginForm() {
        const userInput = await waitForElement('input[name="j_username"]');
        const passInput = await waitForElement('input[name="j_password"]');

        if (userInput) fillField(userInput, 'admin');
        if (passInput) fillField(passInput, 'nimda');

        if (!userInput || !passInput) {
            console.warn('[mpern-toolbar] Login form fields not found for auto-fill.');
        }
    }

    function isLoginPage() {
        return /\/backoffice\/login\.zul/i.test(location.pathname);
    }

    function isBackofficeUrl() {
        return /\/backoffice\//i.test(location.pathname);
    }

    function isHacLoginPage() {
        return /\/hac\/login/i.test(location.pathname);
    }

    /* ------------------------------------------------------------------ */
    /* Agent portal login auto-fill (localhost:4200 / tp-*.scec.co.za)     */
    /* ------------------------------------------------------------------ */

    function isAgentPortalUrl() {
        if (location.hostname === 'localhost' && location.port === '4200') return true;
        return /^tp-[^.]+\.scec\.co\.za$/i.test(location.hostname);
    }

    async function fillAgentLoginForm() {
        const userInput = await waitForElement('input[formcontrolname="userId"]');
        const passInput = await waitForElement('input[formcontrolname="password"]');

        if (userInput) fillField(userInput, 'asagent');
        if (passInput) fillField(passInput, '1234');

        if (!userInput || !passInput) {
            console.warn('[mpern-toolbar] Agent portal login fields not found for auto-fill.');
        }
    }

    /* ------------------------------------------------------------------ */
    /* /authorizationserver/login auto-fill                                */
    /* ------------------------------------------------------------------ */

    function isAuthServerLoginPage() {
        return /\/authorizationserver\/login/i.test(location.pathname);
    }

    async function fillAuthServerLoginForm() {
        const userInput = await waitForElement('input#username[name="username"]');
        const passInput = await waitForElement('input#password[name="password"]');

        if (userInput) fillField(userInput, 'asagent');
        if (passInput) fillField(passInput, '1234');

        if (!userInput || !passInput) {
            console.warn('[mpern-toolbar] authorizationserver login fields not found for auto-fill.');
        }
    }

    onBodyReady(() => {
        if (isBackofficeUrl()) {
            buildToolbar();
            buildNotesPanel();
            if (isLoginPage()) fillLoginForm();
        }
        if (isHacLoginPage()) {
            fillLoginForm();
        }
        if (isAgentPortalUrl()) {
            fillAgentLoginForm();
        }
        if (isAuthServerLoginPage()) {
            fillAuthServerLoginForm();
        }
    });
})();