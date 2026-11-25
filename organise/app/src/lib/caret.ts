// Caret helpers for single-line contenteditable text.

export function caretOffset(el: HTMLElement): number {
  const sel = window.getSelection();
  if (!sel || !sel.rangeCount) return el.textContent?.length ?? 0;
  const r = sel.getRangeAt(0);
  if (!el.contains(r.startContainer)) return el.textContent?.length ?? 0;
  const pre = r.cloneRange();
  pre.selectNodeContents(el);
  pre.setEnd(r.startContainer, r.startOffset);
  return pre.toString().length;
}

export function setCaret(el: HTMLElement, pos: number) {
  const sel = window.getSelection();
  if (!sel) return;
  const range = document.createRange();
  const walker = document.createTreeWalker(el, NodeFilter.SHOW_TEXT);
  let remaining = pos;
  let placed = false;
  let t: Text | null;
  while ((t = walker.nextNode() as Text | null)) {
    if (remaining <= t.length) {
      range.setStart(t, remaining);
      placed = true;
      break;
    }
    remaining -= t.length;
  }
  if (!placed) {
    range.selectNodeContents(el);
    range.collapse(false);
  } else range.collapse(true);
  sel.removeAllRanges();
  sel.addRange(range);
}

/** Is the caret on the first (or last) visual line of the element? */
export function caretOnEdge(el: HTMLElement, which: "first" | "last"): boolean {
  const sel = window.getSelection();
  if (!sel || !sel.rangeCount) return true;
  const rects = sel.getRangeAt(0).getClientRects();
  if (!rects.length) return true;
  const r = rects[0];
  const box = el.getBoundingClientRect();
  const lh = parseFloat(getComputedStyle(el).lineHeight) || 20;
  return which === "first" ? r.top - box.top < lh * 0.8 : box.bottom - r.bottom < lh * 0.8;
}
