// Dragging inside the app (items, sidebar lists, board cards) with pointer events.
// Files dragged in from the OS arrive through Tauri's drag-drop events instead (see App.svelte).
import { app, walk } from "./state.svelte";

export type DragKind = "item" | "list" | "card" | "cat";

export type Target =
  | { kind: "row"; id: string; pos: "before" | "after" | "inside" }
  | { kind: "list"; id: string; pos: "before" | "after" | "into" }
  | { kind: "cat"; id: string }
  | { kind: "column"; id: string; index: number }
  | { kind: "end"; id: string }
  | { kind: "catblock"; id: string; pos: "before" | "after" };

interface DragState {
  kind: DragKind;
  id: string;
  label: string;
  x: number;
  y: number;
  target: Target | null;
}

class Dnd {
  s = $state<DragState | null>(null);
}
export const dnd = new Dnd();

function scrollParent(el: HTMLElement | null): HTMLElement | null {
  while (el && el !== document.body) {
    const st = getComputedStyle(el);
    if ((/(auto|scroll)/.test(st.overflowY) && el.scrollHeight > el.clientHeight + 1) || (/(auto|scroll)/.test(st.overflowX) && el.scrollWidth > el.clientWidth + 1)) return el;
    el = el.parentElement;
  }
  return null;
}

/** Scrolls whatever scrollable area is under (x, y) when the point is near its edge. Returns true if it scrolled. */
export function edgeScroll(x: number, y: number): boolean {
  const sc = scrollParent(document.elementFromPoint(x, y) as HTMLElement | null);
  if (!sc) return false;
  const r = sc.getBoundingClientRect();
  const edge = Math.min(56, r.height / 4);
  const speed = (d: number) => Math.ceil(Math.min(edge, d) / 3);
  let dy = 0;
  let dx = 0;
  if (sc.scrollHeight > sc.clientHeight) {
    if (y < r.top + edge) dy = -speed(r.top + edge - y);
    else if (y > r.bottom - edge) dy = speed(y - (r.bottom - edge));
  }
  if (sc.scrollWidth > sc.clientWidth) {
    if (x < r.left + edge) dx = -speed(r.left + edge - x);
    else if (x > r.right - edge) dx = speed(x - (r.right - edge));
  }
  if (!dx && !dy) return false;
  const before = sc.scrollTop + sc.scrollLeft;
  sc.scrollBy(dx, dy);
  return sc.scrollTop + sc.scrollLeft !== before;
}

/** Keeps scrolling while a drag rests near an edge (the mouse needn't move). */
function scrollLoop() {
  const s = dnd.s;
  if (!s) return;
  if (edgeScroll(s.x, s.y)) s.target = targetAt(s.kind, s.id, s.x, s.y);
  requestAnimationFrame(scrollLoop);
}

/** Starts tracking a possible drag. A press that doesn't move counts as a click. */
export function startDrag(e: PointerEvent, kind: DragKind, id: string, label: string, onClick?: () => void, noDrag = false) {
  if (e.button !== 0) return;
  const sx = e.clientX;
  const sy = e.clientY;
  let started = false;
  let moved = false;
  const move = (ev: PointerEvent) => {
    if (!started) {
      if (Math.hypot(ev.clientX - sx, ev.clientY - sy) < 5) return;
      moved = true;
      if (noDrag) return; // a click target that can't be dragged: moving just cancels the click
      started = true;
      document.body.classList.add("dragging");
      (document.activeElement as HTMLElement | null)?.blur?.();
      dnd.s = { kind, id, label, x: ev.clientX, y: ev.clientY, target: null };
      requestAnimationFrame(scrollLoop);
    }
    if (!dnd.s) return;
    dnd.s.x = ev.clientX;
    dnd.s.y = ev.clientY;
    dnd.s.target = targetAt(kind, id, ev.clientX, ev.clientY);
  };
  const end = (ev: PointerEvent) => {
    window.removeEventListener("pointermove", move);
    window.removeEventListener("pointerup", end);
    window.removeEventListener("pointercancel", end);
    document.body.classList.remove("dragging");
    if (!started) {
      if (ev.type === "pointerup" && !moved) onClick?.();
      return;
    }
    const t = dnd.s?.target ?? null;
    dnd.s = null;
    if (t) drop(kind, id, t);
  };
  window.addEventListener("pointermove", move);
  window.addEventListener("pointerup", end);
  window.addEventListener("pointercancel", end);
}

function contains(rootId: string, id: string): boolean {
  const f = app.find(rootId);
  let found = false;
  if (f) walk(f.node, (n) => (found ||= n.id === id));
  return found;
}

/** On a sidebar list: the top and bottom quarter mean above/below it, the middle means into it. */
function sidebarPos(el: Element, y: number): "before" | "after" | "into" {
  const r = el.getBoundingClientRect();
  const rel = (y - r.top) / r.height;
  return rel < 0.28 ? "before" : rel > 0.72 ? "after" : "into";
}

function third(el: Element, y: number): "before" | "after" | "inside" {
  const r = el.getBoundingClientRect();
  const rel = (y - r.top) / r.height;
  return rel < 0.3 ? "before" : rel > 0.7 ? "after" : "inside";
}

function targetAt(kind: DragKind, id: string, x: number, y: number): Target | null {
  const el = document.elementFromPoint(x, y) as HTMLElement | null;
  if (!el) return null;
  if (kind === "cat") {
    // a whole category block (heading + its lists) in the sidebar
    const block = el.closest<HTMLElement>("[data-cat-block]");
    if (!block) return null;
    let bid = block.dataset.catBlock!;
    let pos: "before" | "after";
    if (bid === "-") {
      // "Other" always stays last: dropping on it means "after the last category"
      const last = app.activeCategories[app.activeCategories.length - 1];
      if (!last) return null;
      bid = last.id;
      pos = "after";
    } else {
      const r = block.getBoundingClientRect();
      pos = y < r.top + r.height / 2 ? "before" : "after";
    }
    return bid === id ? null : { kind: "catblock", id: bid, pos };
  }
  if (kind === "item") {
    const row = el.closest<HTMLElement>("[data-row]");
    if (row) {
      const rid = row.dataset.row!;
      if (contains(id, rid)) return null;
      return { kind: "row", id: rid, pos: third(row, y) };
    }
    const list = el.closest<HTMLElement>("[data-list]");
    if (list) return { kind: "list", id: list.dataset.list!, pos: sidebarPos(list, y) };
    const end = el.closest<HTMLElement>("[data-list-end]");
    if (end) return { kind: "end", id: end.dataset.listEnd! };
    return null;
  }
  if (kind === "list") {
    // another list in the sidebar: above / into / below
    const list = el.closest<HTMLElement>("[data-list]");
    if (list && list.dataset.list !== id) return { kind: "list", id: list.dataset.list!, pos: sidebarPos(list, y) };
    // an item of the open list: the list goes in as a sub-list
    const row = el.closest<HTMLElement>("[data-row]");
    if (row) {
      const rid = row.dataset.row!;
      if (contains(id, rid)) return null;
      return { kind: "row", id: rid, pos: third(row, y) };
    }
    const end = el.closest<HTMLElement>("[data-list-end]");
    if (end && end.dataset.listEnd !== id) return { kind: "end", id: end.dataset.listEnd! };
    const cat = el.closest<HTMLElement>("[data-cat]");
    if (cat) return { kind: "cat", id: cat.dataset.cat! };
  }
  if (kind === "list" || kind === "card") {
    const col = el.closest<HTMLElement>("[data-col]");
    if (col) {
      const cards = Array.from(col.querySelectorAll<HTMLElement>("[data-card]"));
      let index = cards.length;
      for (let i = 0; i < cards.length; i++) {
        const r = cards[i].getBoundingClientRect();
        if (y < r.top + r.height / 2) {
          index = i;
          break;
        }
      }
      return { kind: "column", id: col.dataset.col!, index };
    }
  }
  return null;
}

function drop(kind: DragKind, id: string, t: Target) {
  if (kind === "item") {
    if (t.kind === "row") app.move(id, t.id, t.pos);
    else if (t.kind === "list" && t.pos !== "into") app.move(id, t.id, t.pos); // becomes a list there
    else if (t.kind === "list") {
      const f = app.find(id);
      const list = app.doc.lists.find((l) => l.id === t.id);
      if (f && list && f.list.id === t.id) {
        const last = list.children[list.children.length - 1];
        if (last && last.id !== id) app.move(id, last.id, "after");
      } else app.moveToList(id, t.id);
    } else if (t.kind === "end") {
      const list = app.find(t.id)?.node; // a list, or a sub-list open in a card pop-up
      const last = list?.children[list.children.length - 1];
      if (last && last.id !== id) app.move(id, last.id, "after");
    }
  } else if (kind === "list") {
    if (t.kind === "list") app.move(id, t.id, t.pos === "into" ? "inside" : t.pos);
    else if (t.kind === "row") app.move(id, t.id, t.pos);
    else if (t.kind === "end") app.move(id, t.id, "inside");
    else if (t.kind === "cat") {
      app.setCategory(id, t.id === "-" ? null : t.id);
    } else if (t.kind === "column") {
      app.moveCard(id, t.id, t.index);
    }
  } else if (kind === "cat" && t.kind === "catblock") {
    app.moveCategory(id, t.id, t.pos);
  } else if (kind === "card" && t.kind === "column") {
    app.moveCard(id, t.id, t.index);
  }
}
