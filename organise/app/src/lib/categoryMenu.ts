import { app, PALETTE } from "./state.svelte";
import type { Category, MenuItem } from "./types";

/** The menu for a category (sidebar heading, Settings). */
export function categoryMenu(c: Category): MenuItem[] {
  const n = app.doc.lists.filter((l) => l.category === c.id).length;
  const lists = `${n} ${n === 1 ? "list" : "lists"}`;
  const items: MenuItem[] = [];
  if (!c.archived) {
    const up = app.categoryNeighbour(c.id, -1);
    const down = app.categoryNeighbour(c.id, 1);
    items.push(
      { label: `New list in ${c.name}`, icon: "plus", run: () => app.addList(c.id) },
      { label: "Move up", icon: "arrow-up", disabled: !up, run: () => up && app.moveCategory(c.id, up.id, "before") },
      { label: "Move down", icon: "arrow-down", disabled: !down, run: () => down && app.moveCategory(c.id, down.id, "after") },
      { separator: true, label: "" },
    );
  }
  items.push(
    { label: "Rename", icon: "edit", run: () => app.ask("Rename category", c.name, (v) => app.renameCategory(c.id, v), "", "Rename") },
    { label: "Colour", icon: "palette", submenu: PALETTE.map((p) => ({ label: "Colour", color: p, checked: c.color === p, run: () => app.setCategoryColor(c.id, p) })) },
    { separator: true, label: "" },
    c.archived
      ? { label: "Unarchive", icon: "upload", hint: `Shows it and its ${lists} again`, run: () => app.archiveCategory(c.id, false) }
      : { label: "Archive", icon: "download", hint: `Hides it and its ${lists}; nothing is deleted`, run: () => app.archiveCategory(c.id, true) },
    {
      label: "Fold",
      icon: "chevron-right",
      submenu: [
        { label: "Collapse all categories", icon: "chevron-right", run: () => app.foldAllCategories(true) },
        { label: "Expand all categories", icon: "chevron-down", run: () => app.foldAllCategories(false) },
      ],
    },
    {
      label: "Delete category",
      icon: "trash",
      danger: true,
      hint: n ? `Its ${lists} will have no category` : undefined,
      run: () => app.deleteCategory(c.id),
    },
  );
  return items;
}
