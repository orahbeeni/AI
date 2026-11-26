import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { Category, Column, Doc, Kind, Link, LoadResult, MenuItem, MoveRecord, Node, View } from "./types";

const ALPHABET = "abcdefghijkmnopqrstuvwxyz23456789";
export function newId(prefix: string): string {
  const b = new Uint8Array(7);
  crypto.getRandomValues(b);
  return prefix + Array.from(b, (x) => ALPHABET[x % ALPHABET.length]).join("");
}

export const PALETTE = ["#3b6fd8", "#2f9e62", "#8b5cf6", "#d97706", "#dc2626", "#0891b2", "#db2777", "#64748b"];

export interface Found {
  node: Node;
  parent: Node | null; // null when node is a list
  list: Node;
  siblings: Node[];
  index: number;
  depth: number; // 0 for items directly in a list
}

export interface Prompt {
  title: string;
  value: string;
  placeholder?: string;
  okLabel?: string;
  ok: (v: string) => void;
}

export interface Confirm {
  title: string;
  body?: string;
  okLabel: string;
  danger?: boolean;
  ok: () => void;
}

export interface Toast {
  text: string;
  kind?: "info" | "error";
  action?: { label: string; run: () => void };
}

function emptyDoc(): Doc {
  return { categories: [], columns: [], lists: [], settings: { watched: [], skipHidden: true, autoSearch: true, cardOpens: "popup", theme: "system" } };
}

class AppState {
  doc = $state<Doc>(emptyDoc());
  links = $state<Record<string, Link>>({});
  moves = $state<MoveRecord[]>([]);
  dataDir = $state("");
  host = $state("");
  home = $state("");
  loaded = $state(false);
  loadError = $state<string | null>(null);

  view = $state<View>("list");
  listId = $state<string | null>(null);
  detailsId = $state<string | null>(null);
  /** the board card shown in a pop-up over the board */
  peekId = $state<string | null>(null);
  /** set in a window that shows one card on its own (popped out of the main window) */
  cardWindow = $state<string | null>(null);
  /** how an item gets from a card window to the main window: a system drag (Linux: works on Wayland, which keeps
   *  screen positions to itself) or relaying the pointer's screen position through the app (Windows, macOS) */
  itemDrag = $state<"system" | "relay">(/Linux/.test(navigator.userAgent) && !/Android/.test(navigator.userAgent) ? "system" : "relay");
  focus = $state<{ id: string; caret: "start" | "end" | number } | null>(null);
  flashId = $state<string | null>(null);
  collapsed = $state<Record<string, boolean>>({});
  /** sidebar categories folded shut (by category id; "-" is Other). Remembered on this computer. */
  collapsedCats = $state<Record<string, boolean>>({});
  query = $state("");
  boardFilter = $state<string | null>(null);
  searching = $state(false);

  menu = $state<{ x: number; y: number; items: MenuItem[] } | null>(null);
  prompt = $state<Prompt | null>(null);
  confirm = $state<Confirm | null>(null);
  toast = $state<Toast | null>(null);
  /** where a file drag from the OS would land */
  fileDrop = $state<{ kind: "row" | "list" | "end"; id: string } | null>(null);

  private undoStack: string[] = [];
  private redoStack: string[] = [];
  private lastKey: string | undefined;
  private lastTime = 0;
  private saveTimer: ReturnType<typeof setTimeout> | null = null;
  private dirty = false;
  private toastTimer: ReturnType<typeof setTimeout> | null = null;
  private saving: Promise<void> = Promise.resolve();

  // ---------- loading & saving ----------
  apply(r: LoadResult) {
    this.doc = r.doc;
    this.links = Object.fromEntries(r.links.map((l) => [l.id, l]));
    this.moves = r.moves;
    this.dataDir = r.dataDir;
    this.host = r.host;
    this.home = r.home;
    if (r.itemDrag === "system" || r.itemDrag === "relay") this.itemDrag = r.itemDrag;
    if (!this.listId || !this.doc.lists.some((l) => l.id === this.listId)) this.listId = this.doc.lists[0]?.id ?? null;
    this.loaded = true;
  }

  async load() {
    try {
      const r = await invoke<LoadResult>("load");
      this.apply(r);
      try {
        this.collapsed = JSON.parse(localStorage.getItem("organise.collapsed") || "{}");
        this.collapsedCats = JSON.parse(localStorage.getItem("organise.collapsedCats") || "{}");
      } catch {
        /* private mode etc. */
      }
      return r;
    } catch (e) {
      this.loadError = String(e);
      throw e;
    }
  }

  /** Runs a change to the document with undo support. Changes with the same key in quick succession
   *  (typing in one item) share one undo step. */
  change(fn: () => void, key?: string) {
    const now = Date.now();
    if (!key || key !== this.lastKey || now - this.lastTime > 1500) {
      this.undoStack.push(JSON.stringify($state.snapshot(this.doc)));
      if (this.undoStack.length > 200) this.undoStack.shift();
      this.redoStack = [];
    }
    this.lastKey = key;
    this.lastTime = now;
    fn();
    this.scheduleSave();
  }

  undo() {
    const s = this.undoStack.pop();
    if (!s) return this.say("Nothing to undo");
    this.redoStack.push(JSON.stringify($state.snapshot(this.doc)));
    this.doc = JSON.parse(s);
    this.lastKey = undefined;
    this.scheduleSave();
  }

  redo() {
    const s = this.redoStack.pop();
    if (!s) return;
    this.undoStack.push(JSON.stringify($state.snapshot(this.doc)));
    this.doc = JSON.parse(s);
    this.lastKey = undefined;
    this.scheduleSave();
  }

  scheduleSave() {
    this.dirty = true;
    if (this.saveTimer) clearTimeout(this.saveTimer);
    this.saveTimer = setTimeout(() => this.flush(), 400);
  }

  /** Saves now if there's anything unsaved (when this window loses focus, so another window sees it). */
  flushIfDirty() {
    if (this.dirty) return this.flush();
  }

  async flush() {
    if (this.saveTimer) clearTimeout(this.saveTimer);
    this.saveTimer = null;
    this.dirty = false;
    const doc = $state.snapshot(this.doc);
    this.saving = this.saving.then(() =>
      invoke<void>("save_doc", { doc }).catch((e) => this.say(`Couldn't save: ${e}`, "error")),
    );
    await this.saving;
  }

  /** Another window (a card popped out of the main window, or the main window) saved the document.
   *  Undo history here is dropped: undoing an older step would also undo what was done over there. */
  remoteDoc(doc: Doc) {
    if (JSON.stringify(doc) === JSON.stringify($state.snapshot(this.doc))) return;
    this.doc = doc;
    this.undoStack = [];
    this.redoStack = [];
    this.lastKey = undefined;
    if (this.listId && !this.doc.lists.some((l) => l.id === this.listId)) this.listId = this.doc.lists[0]?.id ?? null;
    if (this.detailsId && !this.find(this.detailsId)) this.detailsId = null;
    if (this.peekId && !this.find(this.peekId)) this.peekId = null;
  }

  // ---------- card windows ----------
  /** Opens a card in a window of its own (size and screen position in logical pixels). */
  async popOut(id: string, size = { w: 760, h: 620 }, at?: { x: number; y: number }) {
    const f = this.find(id);
    if (!f) return;
    await this.flush(); // the new window loads what's saved
    if (this.peekId === id) this.closePeek();
    try {
      await invoke("open_card_window", {
        id,
        title: f.node.text || "Untitled",
        width: Math.round(size.w),
        height: Math.round(size.h),
        x: at ? Math.round(at.x) : null,
        y: at ? Math.round(at.y) : null,
      });
    } catch (e) {
      this.say(String(e), "error");
    }
  }

  /** In a card window: "full" makes the card the main window's view, "dock" puts it back as a pop-up there.
   *  Either way this window closes. */
  async cardToMain(action: "full" | "dock") {
    const id = this.cardWindow;
    if (!id) return;
    await this.flush();
    try {
      await invoke("card_to_main", { action, id });
      await getCurrentWindow().close();
    } catch (e) {
      this.say(String(e), "error");
    }
  }

  // ---------- messages ----------
  say(text: string, kind: "info" | "error" = "info", action?: Toast["action"]) {
    this.toast = { text, kind, action };
    if (this.toastTimer) clearTimeout(this.toastTimer);
    this.toastTimer = setTimeout(() => (this.toast = null), action ? 7000 : 4000);
  }

  /** A short confirmation with an Undo button. */
  done(text: string) {
    this.say(text, "info", { label: "Undo", run: () => this.undo() });
  }

  /** A node's name for messages: “Text” (shortened). */
  q(n: Node | null | undefined): string {
    const t = (n?.text || "Untitled").trim();
    return `“${t.length > 50 ? t.slice(0, 49) + "…" : t}”`;
  }

  openMenu(e: MouseEvent | { clientX: number; clientY: number }, items: MenuItem[]) {
    if ("preventDefault" in e) {
      e.preventDefault();
      e.stopPropagation();
    }
    this.menu = { x: e.clientX, y: e.clientY, items };
  }

  ask(title: string, value: string, ok: (v: string) => void, placeholder = "", okLabel = "OK") {
    this.prompt = { title, value, ok, placeholder, okLabel };
  }

  // ---------- lookups ----------
  find(id: string): Found | null {
    for (const list of this.doc.lists) {
      if (list.id === id) return { node: list, parent: null, list, siblings: this.doc.lists, index: this.doc.lists.indexOf(list), depth: -1 };
      const r = findIn(list, list, 0, id);
      if (r) return r;
    }
    return null;
  }

  get list(): Node | null {
    return this.doc.lists.find((l) => l.id === this.listId) ?? null;
  }

  category(id: string | null | undefined): Category | undefined {
    return this.doc.categories.find((c) => c.id === id);
  }

  /** Categories that aren't archived (for the sidebar, board filters and category pickers). */
  get activeCategories(): Category[] {
    return this.doc.categories.filter((c) => !c.archived);
  }

  /** Is this list (or the list holding this node) in an archived category? */
  isArchived(id: string): boolean {
    const f = this.find(id);
    return !!f && !!this.category(f.list.category)?.archived;
  }

  categoryOfNode(id: string): Category | undefined {
    const f = this.find(id);
    return f ? this.category(f.list.category) : undefined;
  }

  columnOf(nodeId: string): Column | undefined {
    return this.doc.columns.find((c) => c.cards.includes(nodeId));
  }

  missingLinks(): Link[] {
    const used = new Set<string>();
    for (const l of this.doc.lists) walk(l, (n) => n.links.forEach((x) => used.add(x)));
    return Object.values(this.links).filter((l) => l.status !== "ok" && used.has(l.id));
  }

  /** How many different links the lists use. */
  linkTotal(): number {
    const used = new Set<string>();
    for (const l of this.doc.lists) walk(l, (n) => n.links.forEach((x) => used.add(x)));
    return used.size;
  }

  /** Items (with their list) that use a link. */
  usersOf(linkId: string): { node: Node; list: Node }[] {
    const out: { node: Node; list: Node }[] = [];
    for (const l of this.doc.lists) walk(l, (n) => n.links.includes(linkId) && out.push({ node: n, list: l }));
    return out;
  }

  // ---------- navigation ----------
  openList(id: string, focusItem?: string) {
    this.peekId = null;
    const f = this.find(focusItem ?? id);
    this.listId = f ? f.list.id : id;
    // unfold its category in the sidebar so you can see where you are
    const list = f?.list ?? this.doc.lists.find((l) => l.id === id);
    if (list && this.collapsedCats[this.groupOf(list)]) this.toggleCategory(this.groupOf(list));
    this.view = "list";
    if (focusItem && f) {
      // make sure it's visible
      let p = this.find(focusItem);
      while (p && p.parent) {
        this.collapsed[p.parent.id] = false;
        p = this.find(p.parent.id);
      }
      this.flashId = focusItem;
      setTimeout(() => (this.flashId = null), 1600);
      this.focus = { id: focusItem, caret: "end" };
    }
  }

  /** A board card was clicked: show it in a pop-up over the board, or go straight to it,
   *  as set in Settings › Board (or as asked for by `how`). */
  openCard(id: string, how?: "popup" | "page") {
    if (!this.find(id)) return;
    if ((how ?? this.doc.settings.cardOpens ?? "popup") === "popup") this.peekId = id;
    else this.openFull(id);
  }

  /** Makes a list the main view. A sub-list opens inside its list, scrolled to and highlighted. */
  openFull(id: string) {
    const f = this.find(id);
    if (!f) return;
    this.openList(f.list.id, f.parent ? id : undefined);
  }

  /** Closes the card pop-up (and the details of anything in it). */
  closePeek() {
    const f = this.peekId ? this.find(this.peekId) : null;
    if (f && this.detailsId) walk(f.node, (n) => n.id === this.detailsId && (this.detailsId = null));
    this.peekId = null;
  }

  toggleCollapse(id: string) {
    this.collapsed[id] = !this.collapsed[id];
    try {
      localStorage.setItem("organise.collapsed", JSON.stringify(this.collapsed));
    } catch {
      /* ignore */
    }
  }

  // ---------- lists ----------
  addList(categoryId?: string | null) {
    const current = this.list && !this.isArchived(this.list.id) ? this.list.category : undefined;
    const cat = categoryId ?? current ?? this.activeCategories[0]?.id ?? null;
    const list: Node = { id: newId("L"), text: "", kind: "task", done: false, links: [], notes: "", children: [], category: cat };
    this.change(() => {
      this.doc.lists.push(list);
    });
    this.openList(list.id);
    this.focus = { id: list.id, caret: "start" };
    return list;
  }

  deleteNode(id: string, notify = true) {
    const f = this.find(id);
    if (!f) return;
    const msg = `Deleted ${f.parent ? "" : "the list "}${this.q(f.node)}`;
    this.change(() => {
      f.siblings.splice(f.index, 1);
      const gone = new Set<string>();
      walk(f.node, (n) => gone.add(n.id));
      for (const c of this.doc.columns) c.cards = c.cards.filter((x) => !gone.has(x));
    });
    if (notify) this.done(msg);
    if (this.detailsId && !this.find(this.detailsId)) this.detailsId = null;
    if (this.peekId && !this.find(this.peekId)) this.peekId = null;
    if (!f.parent && this.listId === id) this.listId = this.doc.lists[Math.max(0, f.index - 1)]?.id ?? null;
  }

  /** The sidebar group a list shows in: its category, or "-" (Other) when it has none. */
  toggleCategory(id: string) {
    this.collapsedCats[id] = !this.collapsedCats[id];
    this.saveCats();
  }

  /** Folds every sidebar category shut, or opens them all. */
  foldAllCategories(fold: boolean) {
    const ids = [...this.activeCategories.map((c) => c.id), "-"];
    this.collapsedCats = Object.fromEntries(ids.map((id) => [id, fold]));
    this.saveCats();
  }

  private saveCats() {
    try {
      localStorage.setItem("organise.collapsedCats", JSON.stringify(this.collapsedCats));
    } catch {
      /* not remembered, still works */
    }
  }

  groupOf(l: Node): string {
    return this.doc.categories.some((c) => c.id === l.category) ? (l.category as string) : "-";
  }

  /** Index of the next list above (-1) or below (1) in the same sidebar group, or -1. */
  private neighbour(id: string, dir: -1 | 1): number {
    const lists = this.doc.lists;
    const i = lists.findIndex((l) => l.id === id);
    if (i < 0) return -1;
    const g = this.groupOf(lists[i]);
    for (let j = i + dir; j >= 0 && j < lists.length; j += dir) if (this.groupOf(lists[j]) === g) return j;
    return -1;
  }

  canMoveList(id: string, dir: -1 | 1): boolean {
    return this.neighbour(id, dir) >= 0;
  }

  /** Moves a list up or down within its category in the sidebar. */
  moveList(id: string, dir: -1 | 1) {
    const j = this.neighbour(id, dir);
    if (j < 0) return;
    this.change(() => {
      const i = this.doc.lists.findIndex((l) => l.id === id);
      const [l] = this.doc.lists.splice(i, 1);
      this.doc.lists.splice(j, 0, l);
    });
  }

  setCategory(listId: string, cat: string | null) {
    const l = this.doc.lists.find((x) => x.id === listId);
    if (!l || (l.category ?? null) === cat) return;
    this.change(() => (l.category = cat));
    const c = this.category(cat);
    this.done(c ? `${this.q(l)} is now in ${c.name}` : `${this.q(l)} has no category now`);
  }

  // ---------- items ----------
  setText(id: string, text: string) {
    const f = this.find(id);
    if (!f || f.node.text === text) return;
    this.change(() => (f.node.text = text), "text:" + id);
  }

  setNotes(id: string, notes: string) {
    const f = this.find(id);
    if (!f || f.node.notes === notes) return;
    this.change(() => (f.node.notes = notes), "notes:" + id);
  }

  newItem(text = "", kind?: Kind): Node {
    return { id: newId("i"), text, kind: kind ?? "task", done: false, links: [], notes: "", children: [] };
  }

  /** Adds an item to the end of a list (or as the last child of an item). */
  append(parentId: string, text = "", links: string[] = []): Node | null {
    const f = this.find(parentId);
    if (!f) return null;
    const last = f.node.children[f.node.children.length - 1];
    const item = { ...this.newItem(text, last?.kind), links };
    this.change(() => f.node.children.push(item));
    return item;
  }

  /** Enter in an item: splits it at the caret. The new item goes below (or becomes the first child
   *  when the item has open children). */
  split(id: string, caret: number) {
    const f = this.find(id);
    if (!f || !f.parent) return;
    const text = f.node.text;
    const before = text.slice(0, caret);
    const after = text.slice(caret);
    const item = this.newItem(after, f.node.kind);
    this.change(() => {
      if (caret === 0 && text.length > 0) {
        // Enter at the start: new empty item above, keep this one
        item.text = "";
        f.siblings.splice(f.index, 0, item);
        return;
      }
      f.node.text = before;
      if (f.node.children.length && !this.collapsed[f.node.id]) f.node.children.unshift(item);
      else f.siblings.splice(f.index + 1, 0, item);
    });
    this.focus = { id: caret === 0 && text.length > 0 ? id : item.id, caret: "start" };
  }

  /** Backspace at the start of an item: it joins the end of the row above it (`prevId`, the previous row
   *  on screen) and its marker goes. Its children, links and notes go along with the text. The first item of
   *  a list has nothing above it and never joins. Returns false when nothing happened. */
  joinUp(id: string, prevId: string): boolean {
    const f = this.find(id);
    const p = this.find(prevId);
    if (!f || !p || !f.parent || !p.parent || id === prevId) return false;
    if (this.columnOf(id)) {
      this.say(`${this.q(f.node)} is a card on the board. Take it off the board to join it to the item above.`);
      return false;
    }
    const n = f.node;
    const into = p.node;
    const at = into.text.length;
    const intoParent = into.id === f.parent.id; // the first child joining the item it's under
    this.change(() => {
      into.text += n.text;
      for (const l of n.links) if (!into.links.includes(l)) into.links.push(l);
      if (n.notes.trim()) into.notes = into.notes.trim() ? `${into.notes.trimEnd()}\n\n${n.notes}` : n.notes;
      // its children stay where they are on screen: under the joined item
      f.siblings.splice(f.index, 1, ...(intoParent ? n.children : []));
      if (!intoParent && n.children.length) {
        into.children.push(...n.children);
        this.collapsed[into.id] = false;
      }
    });
    if (this.detailsId === id) this.detailsId = prevId;
    this.focus = { id: prevId, caret: at };
    return true;
  }

  indent(id: string) {
    const f = this.find(id);
    if (!f || !f.parent || f.index === 0) return;
    const prev = f.siblings[f.index - 1];
    this.change(() => {
      f.siblings.splice(f.index, 1);
      prev.children.push(f.node);
      this.collapsed[prev.id] = false;
    });
  }

  outdent(id: string) {
    const f = this.find(id);
    if (!f || !f.parent || f.parent.id === f.list.id) return;
    const pf = this.find(f.parent.id);
    if (!pf) return;
    this.change(() => {
      f.siblings.splice(f.index, 1);
      pf.siblings.splice(pf.index + 1, 0, f.node);
    });
  }

  shift(id: string, dir: -1 | 1) {
    const f = this.find(id);
    if (!f) return;
    const j = f.index + dir;
    if (j < 0 || j >= f.siblings.length) return;
    this.change(() => {
      const [n] = f.siblings.splice(f.index, 1);
      f.siblings.splice(j, 0, n);
    });
  }

  /** Moves a node before/after/inside another. A list moved inside a list (or next to an item) becomes a
   *  sub-list; an item moved next to a list becomes a list. The node keeps its id, so a board card for it
   *  stays in its column and place. */
  move(id: string, targetId: string, pos: "before" | "after" | "inside"): boolean {
    if (id === targetId) return false;
    const f = this.find(id);
    const t = this.find(targetId);
    if (!f || !t) return false;
    let inside = false;
    walk(f.node, (n) => (inside ||= n.id === targetId));
    if (inside) return false; // into itself
    const wasList = !f.parent;
    const becomesList = pos !== "inside" && !t.parent;
    const category = becomesList ? (t.node.category ?? null) : null;
    this.change(() => {
      f.siblings.splice(f.index, 1);
      const t2 = this.find(targetId)!;
      if (pos === "inside") {
        t2.node.children.push(f.node);
        this.collapsed[t2.node.id] = false;
      } else {
        t2.siblings.splice(t2.index + (pos === "after" ? 1 : 0), 0, f.node);
      }
      if (becomesList) f.node.category = category;
      else delete f.node.category;
    });
    const name = `“${f.node.text || "Untitled"}”`;
    const undo = { label: "Undo", run: () => this.undo() };
    if (wasList && !becomesList) {
      const nf = this.find(id);
      if (nf && this.listId === id) this.openList(nf.list.id, id);
      this.say(`${name} is now inside ${nf?.list.text || "Untitled"}. Its board card stays where it was.`, "info", undo);
    } else if (!wasList && becomesList) {
      this.say(`${name} is now a list`, "info", undo);
    }
    return true;
  }

  /** An item (or sub-list) becomes a list of its own, just below the list it was in. */
  promote(id: string) {
    const f = this.find(id);
    if (f && f.parent) this.move(id, f.list.id, "after");
  }

  /** Moves an item to the end of another list. */
  moveToList(id: string, listId: string): boolean {
    const f = this.find(id);
    const l = this.doc.lists.find((x) => x.id === listId);
    if (!f || !l || !f.parent || f.list.id === listId) return false;
    this.change(() => {
      f.siblings.splice(f.index, 1);
      l.children.push(f.node);
    });
    this.say(`Moved to ${l.text || "Untitled"}`, "info", { label: "Undo", run: () => this.undo() });
    return true;
  }

  toggleDone(id: string) {
    const f = this.find(id);
    if (!f) return;
    this.change(() => {
      if (f.node.kind === "bullet") f.node.kind = "task";
      f.node.done = !f.node.done;
    });
  }

  toggleKind(id: string) {
    const f = this.find(id);
    if (!f) return;
    this.change(() => {
      f.node.kind = f.node.kind === "task" ? "bullet" : "task";
      if (f.node.kind === "bullet") f.node.done = false;
    });
  }

  // ---------- links ----------
  async addPaths(paths: string[]): Promise<Link[]> {
    const links = await invoke<Link[]>("add_paths", { paths });
    for (const l of links) this.links[l.id] = l;
    return links;
  }

  async addUrl(url: string, title?: string): Promise<Link> {
    const l = await invoke<Link>("add_url", { url, title: title ?? null });
    this.links[l.id] = l;
    return l;
  }

  attach(nodeId: string, linkIds: string[]) {
    const f = this.find(nodeId);
    if (!f) return;
    const add = linkIds.filter((x) => !f.node.links.includes(x));
    if (!add.length) return;
    this.change(() => f.node.links.push(...add));
  }

  detach(nodeId: string, linkId: string) {
    const f = this.find(nodeId);
    if (!f) return;
    this.change(() => (f.node.links = f.node.links.filter((x) => x !== linkId)));
    this.done(`Removed the link to ${this.links[linkId]?.name ?? "the file"}. The file itself is untouched.`);
  }

  removeLinkEverywhere(linkId: string) {
    const n = this.usersOf(linkId).length;
    this.change(() => {
      for (const l of this.doc.lists) walk(l, (n) => (n.links = n.links.filter((x) => x !== linkId)));
    });
    this.done(`Removed the link to ${this.links[linkId]?.name ?? "the file"} from ${n} ${n === 1 ? "item" : "items"}`);
  }

  updateLinks(list: Link[]) {
    for (const l of list) this.links[l.id] = l;
  }

  async openLink(id: string) {
    try {
      await invoke("open_link", { id });
    } catch (e) {
      this.say(String(e), "error");
    }
  }

  async revealLink(id: string) {
    try {
      await invoke("reveal_link", { id });
    } catch (e) {
      this.say(String(e), "error");
    }
  }

  async relink(id: string, path: string) {
    try {
      const l = await invoke<Link>("relink", { id, path });
      this.links[l.id] = l;
      this.say(`Now linked to ${l.name}`);
    } catch (e) {
      this.say(String(e), "error");
    }
  }

  // ---------- board ----------
  putOnBoard(nodeId: string, colId?: string) {
    const col = this.doc.columns.find((c) => c.id === colId) ?? this.doc.columns[0];
    if (!col) return this.say("Add a board column first (Settings › Board)");
    const was = this.columnOf(nodeId);
    if (was?.id === col.id) return this.say(`${this.q(this.find(nodeId)?.node)} is already in ${col.name}`);
    this.change(() => {
      for (const c of this.doc.columns) c.cards = c.cards.filter((x) => x !== nodeId);
      col.cards.push(nodeId);
    });
    const name = this.q(this.find(nodeId)?.node);
    this.done(was ? `Moved ${name} to ${col.name}` : `${name} is on the board in ${col.name}`);
  }

  takeOffBoard(nodeId: string) {
    if (!this.columnOf(nodeId)) return;
    this.change(() => {
      for (const c of this.doc.columns) c.cards = c.cards.filter((x) => x !== nodeId);
    });
    this.done(`Took ${this.q(this.find(nodeId)?.node)} off the board`);
  }

  /** Board drags call this without a message (the card visibly moves); menus pass notify. */
  moveCard(nodeId: string, colId: string, index: number, notify = false) {
    const col = this.doc.columns.find((c) => c.id === colId);
    if (!col) return;
    this.change(() => {
      for (const c of this.doc.columns) {
        const i = c.cards.indexOf(nodeId);
        if (i >= 0) {
          c.cards.splice(i, 1);
          if (c === col && i < index) index--;
        }
      }
      col.cards.splice(Math.max(0, Math.min(index, col.cards.length)), 0, nodeId);
    });
    if (notify) this.done(`Moved ${this.q(this.find(nodeId)?.node)} to ${col.name}`);
  }

  /** A drag onto a board column: moves the card there, or puts something new on the board (and says so,
   *  since it may have come from a pop-up or the tray). */
  placeCard(nodeId: string, colId: string, index: number) {
    const was = this.columnOf(nodeId);
    const col = this.doc.columns.find((c) => c.id === colId);
    if (!col || !this.find(nodeId)) return;
    this.moveCard(nodeId, colId, index);
    if (!was) this.done(`${this.q(this.find(nodeId)?.node)} is on the board in ${col.name}`);
  }

  /** Lists and sub-lists (items with items under them) that have no card on the board, list by list. */
  notOnBoard(categoryId: string | null): { node: Node; list: Node; sub: boolean }[] {
    const on = new Set(this.doc.columns.flatMap((c) => c.cards));
    const out: { node: Node; list: Node; sub: boolean }[] = [];
    for (const list of this.doc.lists) {
      const cat = this.category(list.category);
      if (cat?.archived || (categoryId && list.category !== categoryId)) continue;
      walk(list, (n) => {
        if (on.has(n.id)) return;
        if (n === list) out.push({ node: n, list, sub: false });
        else if (n.children.length) out.push({ node: n, list, sub: true });
      });
    }
    return out;
  }

  addColumn(name: string) {
    const n = name.trim();
    if (!n) return;
    this.change(() => this.doc.columns.push({ id: newId("c"), name: n, color: null, cards: [] }));
    this.done(`Added the “${n}” column`);
  }

  renameColumn(id: string, name: string) {
    const c = this.doc.columns.find((x) => x.id === id);
    if (!c || !name.trim()) return;
    this.change(() => (c.name = name.trim()));
  }

  deleteColumn(id: string) {
    const i = this.doc.columns.findIndex((x) => x.id === id);
    if (i < 0) return;
    const c = this.doc.columns[i];
    if (c.cards.length) return this.say(`The “${c.name}” column still has ${c.cards.length} ${c.cards.length === 1 ? "card" : "cards"}. Move them out first.`);
    this.change(() => this.doc.columns.splice(i, 1));
    this.done(`Deleted the “${c.name}” column`);
  }

  moveColumn(id: string, dir: -1 | 1) {
    const i = this.doc.columns.findIndex((x) => x.id === id);
    const j = i + dir;
    if (i < 0 || j < 0 || j >= this.doc.columns.length) return;
    this.change(() => {
      const [c] = this.doc.columns.splice(i, 1);
      this.doc.columns.splice(j, 0, c);
    });
  }

  setColumnColor(id: string, color: string | null) {
    const c = this.doc.columns.find((x) => x.id === id);
    if (c) this.change(() => (c.color = color));
  }

  // ---------- categories ----------
  addCategory(name: string): Category | null {
    const n = name.trim();
    if (!n) return null;
    const used = new Set(this.doc.categories.map((c) => c.color));
    const c: Category = { id: newId("k"), name: n, color: PALETTE.find((p) => !used.has(p)) ?? PALETTE[0] };
    this.change(() => this.doc.categories.push(c));
    return c;
  }

  renameCategory(id: string, name: string) {
    const c = this.doc.categories.find((x) => x.id === id);
    if (c && name.trim()) this.change(() => (c.name = name.trim()));
  }

  setCategoryColor(id: string, color: string) {
    const c = this.doc.categories.find((x) => x.id === id);
    if (c) this.change(() => (c.color = color));
  }

  /** Moves a category before/after another (sidebar order). */
  moveCategory(id: string, targetId: string, pos: "before" | "after") {
    if (id === targetId) return;
    const cats = this.doc.categories;
    if (!cats.some((c) => c.id === id) || !cats.some((c) => c.id === targetId)) return;
    this.change(() => {
      const [c] = cats.splice(cats.findIndex((x) => x.id === id), 1);
      cats.splice(cats.findIndex((x) => x.id === targetId) + (pos === "after" ? 1 : 0), 0, c);
    });
  }

  /** The next visible category above (-1) or below (1), if any. */
  categoryNeighbour(id: string, dir: -1 | 1): Category | undefined {
    const act = this.activeCategories;
    const i = act.findIndex((c) => c.id === id);
    return i < 0 ? undefined : act[i + dir];
  }

  /** Adds a category from the sidebar's "New category" button. */
  newCategory() {
    this.ask("New category", "", (v) => {
      const c = this.addCategory(v);
      if (c) this.done(`Added the “${c.name}” category. Use its + to add a list.`);
    }, "e.g. Side projects", "Add");
  }

  /** Archiving hides a category and its lists from the sidebar and the board. Nothing is deleted. */
  archiveCategory(id: string, archived = true) {
    const c = this.category(id);
    if (!c || !!c.archived === archived) return;
    const n = this.doc.lists.filter((l) => l.category === id).length;
    const lists = `${n} ${n === 1 ? "list" : "lists"}`;
    this.change(() => {
      if (archived) c.archived = true;
      else delete c.archived;
    });
    if (archived && this.list?.category === id) {
      this.listId = this.doc.lists.find((l) => !this.isArchived(l.id))?.id ?? null;
      this.detailsId = null;
    }
    if (archived && this.boardFilter === id) this.boardFilter = null;
    this.done(
      archived
        ? `Archived “${c.name}”. Its ${lists} ${n === 1 ? "is" : "are"} hidden from the sidebar and the board.`
        : `“${c.name}” is back, with its ${lists}`,
    );
  }

  deleteCategory(id: string) {
    const c = this.category(id);
    const n = this.doc.lists.filter((l) => l.category === id).length;
    this.change(() => {
      this.doc.categories = this.doc.categories.filter((c) => c.id !== id);
      for (const l of this.doc.lists) if (l.category === id) l.category = null;
    });
    if (c) this.done(`Deleted the “${c.name}” category${n ? `. ${n} ${n === 1 ? "list has" : "lists have"} no category now.` : ""}`);
  }
}

function findIn(node: Node, list: Node, depth: number, id: string): Found | null {
  const kids = node.children;
  for (let i = 0; i < kids.length; i++) {
    if (kids[i].id === id) return { node: kids[i], parent: node, list, siblings: kids, index: i, depth };
    const r = findIn(kids[i], list, depth + 1, id);
    if (r) return r;
  }
  return null;
}

export function walk(n: Node, f: (n: Node) => void) {
  f(n);
  for (const c of n.children) walk(c, f);
}

/** checked / total over the tasks below a node */
export function progress(n: Node): { done: number; total: number } {
  let done = 0;
  let total = 0;
  for (const c of n.children)
    walk(c, (x) => {
      if (x.kind === "task") {
        total++;
        if (x.done) done++;
      }
    });
  return { done, total };
}

export function linkCount(n: Node): number {
  let k = 0;
  walk(n, (x) => (k += x.links.length));
  return k;
}

export function shortPath(p: string | undefined, home: string): string {
  if (!p) return "";
  if (home && (p === home || p.startsWith(home + "/"))) return "~" + p.slice(home.length);
  return p;
}

export function dirOf(p: string | undefined): string {
  if (!p) return "";
  const i = p.lastIndexOf("/");
  return i > 0 ? p.slice(0, i) : "/";
}

export function ago(secs: number): string {
  const d = Date.now() / 1000 - secs;
  if (d < 60) return "just now";
  if (d < 3600) return `${Math.floor(d / 60)} min ago`;
  if (d < 86400) return `${Math.floor(d / 3600)} h ago`;
  const date = new Date(secs * 1000);
  if (d < 7 * 86400) return date.toLocaleDateString(undefined, { weekday: "short" });
  return date.toLocaleDateString(undefined, { day: "numeric", month: "short" });
}

export function isUrl(s: string): boolean {
  return /^(https?:\/\/|mailto:)\S+$/i.test(s.trim());
}

export const app = new AppState();
