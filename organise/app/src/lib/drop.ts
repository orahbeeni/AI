// Files and folders dragged in from the file manager.
import { app } from "./state.svelte";

export type FileTarget = { kind: "row" | "list" | "end"; id: string } | null;

/** What's under a point (CSS pixels) for a file drop. */
export function fileTargetAt(x: number, y: number): FileTarget {
  const el = document.elementFromPoint(x, y) as HTMLElement | null;
  if (!el) return null;
  const row = el.closest<HTMLElement>("[data-row]");
  if (row) return { kind: "row", id: row.dataset.row! };
  const list = el.closest<HTMLElement>("[data-list]");
  if (list) return { kind: "list", id: list.dataset.list! };
  const end = el.closest<HTMLElement>("[data-list-end]");
  if (end) return { kind: "end", id: end.dataset.listEnd! };
  // anywhere else on a card pop-up or an open list: add at the end
  const peek = el.closest<HTMLElement>("[data-peek]");
  if (peek) return { kind: "end", id: peek.dataset.peek! };
  if (app.view === "list" && app.listId && el.closest("main")) return { kind: "end", id: app.listId };
  return null;
}

function itemName(name: string): string {
  const i = name.lastIndexOf(".");
  return i > 0 ? name.slice(0, i) : name;
}

export async function handleDrop(paths: string[], t: FileTarget) {
  if (!paths.length) return;
  if (!t) {
    app.say("Drop files onto an item, below a list, or onto a list in the sidebar");
    return;
  }
  let links;
  try {
    links = await app.addPaths(paths);
  } catch (e) {
    app.say(String(e), "error");
    return;
  }
  const what = links.length === 1 ? links[0].name : `${links.length} files`;
  if (t.kind === "row") {
    app.attach(t.id, links.map((l) => l.id));
    app.say(`Linked ${what}. The file stays where it is.`, "info", { label: "Undo", run: () => app.undo() });
    return;
  }
  const list = t.kind === "list" ? app.doc.lists.find((l) => l.id === t.id) : app.find(t.id)?.node;
  if (!list) return;
  const last = list.children[list.children.length - 1];
  const items = links.map((l) => ({ ...app.newItem(itemName(l.name), last?.kind), links: [l.id] }));
  app.change(() => list.children.push(...items));
  app.say(
    t.kind === "list" ? `Added ${links.length === 1 ? "an item" : `${links.length} items`} to ${list.text || "Untitled"}` : `Linked ${what}`,
    "info",
    { label: "Undo", run: () => app.undo() },
  );
}
