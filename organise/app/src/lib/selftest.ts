// Runs with ORGANISE_SELFTEST=1: drives the app through the same functions the UI uses, including a real
// file move on disk, and reports to the terminal. Uses a throwaway data folder (ORGANISE_DIR).
import { invoke } from "@tauri-apps/api/core";
import { app, walk } from "./state.svelte";
import { handleDrop } from "./drop";
import { caretOffset, setCaret } from "./caret";

const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));
async function until(fn: () => boolean, ms = 10000) {
  const t = Date.now();
  while (Date.now() - t < ms) {
    if (fn()) return true;
    await wait(100);
  }
  return false;
}

export async function selftest(file: string) {
  const log: string[] = [];
  let ok = true;
  const check = (name: string, cond: boolean, extra = "") => {
    log.push(`${cond ? "ok  " : "FAIL"} ${name}${!cond && extra ? `  (${extra})` : ""}`);
    if (!cond) ok = false;
  };
  try {
    check("starter list loaded", app.doc.lists.length >= 1);
    const list = app.addList("work");
    app.setText(list.id, "Self test list");
    const a = app.append(list.id, "Alpha")!;
    const b = app.append(list.id, "Bravo")!;
    const c = app.append(list.id, "Charlie")!;
    app.indent(b.id);
    check("Tab nests Bravo under Alpha", app.find(b.id)?.parent?.id === a.id);
    app.outdent(b.id);
    check("Shift+Tab brings it back", app.find(b.id)?.parent?.id === list.id && app.find(b.id)?.index === 1);
    app.split(c.id, 4);
    const l2 = app.list!;
    check("Enter splits at the caret", app.find(c.id)?.node.text === "Char" && l2.children[3]?.text === "lie", JSON.stringify(l2.children.map((x) => x.text)));
    app.toggleDone(a.id);
    check("checkbox toggles", app.find(a.id)!.node.done);
    app.move(c.id, a.id, "inside");
    check("drag onto an item nests it", app.find(c.id)?.parent?.id === a.id);
    app.undo();
    check("undo", app.find(c.id)?.parent?.id === list.id);
    app.redo();
    check("redo", app.find(c.id)?.parent?.id === a.id);
    check("can't drag an item into its own child", !app.move(a.id, c.id, "inside"));

    // reordering lists in the sidebar (within a category)
    const second = app.addList("work");
    app.setText(second.id, "Second work list");
    const pos = (id: string) => app.doc.lists.findIndex((l) => l.id === id);
    check("a new list goes to the bottom of its category", pos(second.id) > pos(list.id));
    check("the top list of a category can't move up", !app.canMoveList(list.id, -1));
    app.moveList(second.id, -1);
    check("Move up puts it above the other list", pos(second.id) < pos(list.id));
    app.moveList(second.id, 1);
    check("Move down puts it back", pos(second.id) > pos(list.id));
    app.openList(list.id);

    // a list (that's on the board) moved inside another list keeps its card
    app.append(second.id, "Second's item");
    app.putOnBoard(second.id, "review");
    const reviewIdx = () => app.doc.columns.find((x) => x.id === "review")!.cards.indexOf(second.id);
    const idxBefore = reviewIdx();
    check("a list can't go inside itself", !app.move(second.id, second.id, "inside"));
    app.move(second.id, list.id, "inside");
    check("a list dropped into another list becomes a sub-list", app.find(second.id)?.parent?.id === list.id);
    check("its items come with it", app.find(second.id)?.node.children[0]?.text === "Second's item");
    check("it leaves the sidebar", !app.doc.lists.some((l) => l.id === second.id));
    check("its board card stays in the same column and place", app.columnOf(second.id)?.id === "review" && reviewIdx() === idxBefore);
    app.promote(second.id);
    check("Turn into a list makes it a list again", app.doc.lists.some((l) => l.id === second.id) && app.find(second.id)?.node.category === "work");
    check("…still on the board", app.columnOf(second.id)?.id === "review" && reviewIdx() === idxBefore);
    app.move(second.id, list.id, "inside");
    app.openList(list.id);

    await handleDrop([file], { kind: "row", id: b.id });
    const lid = app.find(b.id)!.node.links[0];
    check("dropping a file on an item links it", !!lid && app.links[lid]?.path === file, JSON.stringify(app.links[lid]));
    await handleDrop([file], { kind: "end", id: list.id });
    const last = app.list!.children[app.list!.children.length - 1];
    check("dropping below the list makes an item named after the file", last.text === "Quarterly report" && last.links[0] === lid, last.text);

    await app.flush();
    const md = await invoke<string>("selftest_read_list", { titleSlug: "self-test-list" });
    check("list file holds the link", md.includes(`[Quarterly report.txt](link:${lid})`), md);
    check("list file holds the nesting", /\n- \[x\] Alpha \^\w+\n {2}- \[ \] Char \^/.test(md), md);
    check("the sub-list and its items are in the list file", md.includes(`- [ ] Second work list ^${second.id}\n  - [ ] Second's item ^`), md);

    // rename in the same folder: the watcher sees both names at once
    const renamed = file.replace("Quarterly report.txt", "Q3 report.txt");
    await invoke("selftest_move", { from: file, to: renamed });
    check("renamed file: link follows", await until(() => app.links[lid]?.path === renamed), JSON.stringify(app.links[lid]));
    // move into a folder nobody watches: found by searching from the old place
    const moved = renamed.replace("/selftest-area/", "/selftest-area/archive/2026/");
    await invoke("selftest_move", { from: renamed, to: moved });
    check("moved file: link follows", await until(() => app.links[lid]?.path === moved, 15000), JSON.stringify(app.links[lid]));
    check("the move is listed", app.moves[0]?.to === moved, JSON.stringify(app.moves[0]));
    await app.flush();
    const md2 = await invoke<string>("selftest_read_list", { titleSlug: "self-test-list" });
    check("list file shows the new name", md2.includes(`[Q3 report.txt](link:${lid})`), md2);

    app.putOnBoard(list.id, "doing");
    check("list goes on the board", app.columnOf(list.id)?.id === "doing");
    check("…and says so, with Undo", !!app.toast?.text.includes("is on the board in In progress") && app.toast?.action?.label === "Undo", app.toast?.text);
    app.takeOffBoard(list.id);
    check("taking it off says so", !!app.toast?.text.includes("off the board") && !app.columnOf(list.id), app.toast?.text);
    app.toast?.action?.run();
    check("the toast's Undo puts it back", app.columnOf(list.id)?.id === "doing");
    const doomed = app.append(list.id, "Delete me")!;
    app.deleteNode(doomed.id);
    check("deleting an item says so", !!app.toast?.text.includes("Deleted “Delete me”"), app.toast?.text);
    app.toast?.action?.run();
    check("…and Undo brings it back", !!app.find(doomed.id));
    app.deleteNode(doomed.id, false);
    app.deleteColumn("doing");
    check("a column with cards can't be deleted", app.doc.columns.some((x) => x.id === "doing"));
    app.moveCard(list.id, "done", 0);
    check("card moves to another column", app.columnOf(list.id)?.id === "done");
    // archiving a category hides it and its lists, keeps their board cards, and comes back intact
    const inSidebar = () => !!document.querySelector(`[data-list="${list.id}"]`);
    await wait(50);
    check("the list shows in the sidebar", inSidebar());
    app.archiveCategory("work");
    await wait(50);
    check("archiving hides the category", !app.activeCategories.some((c) => c.id === "work"));
    check("…and its lists leave the sidebar", app.isArchived(list.id) && !inSidebar());
    check("…and says so with Undo", !!app.toast?.text.startsWith("Archived “Work”") && app.toast?.action?.label === "Undo", app.toast?.text);
    check("the open list switches to one that's still visible", !app.list || !app.isArchived(app.list.id));
    check("its board card keeps its place (just hidden)", app.columnOf(list.id)?.id === "done");
    const fresh = app.addList();
    check("a new list doesn't go into the archived category", fresh.category !== "work", String(fresh.category));
    app.deleteNode(fresh.id, false);
    app.archiveCategory("work", false);
    await wait(50);
    check("unarchiving brings the category and its lists back", app.activeCategories.some((c) => c.id === "work") && !app.isArchived(list.id) && inSidebar());
    app.openList(list.id);

    // folding a category in the sidebar
    app.toggleCategory("work");
    await wait(50);
    check("folding a category hides its lists and shows a count", !inSidebar() && !!document.querySelector('[data-cat-block="work"] .n'));
    app.openList(list.id);
    await wait(50);
    check("opening one of its lists unfolds it", inSidebar() && !app.collapsedCats["work"]);
    app.foldAllCategories(true);
    await wait(50);
    check("Collapse all folds every category", !document.querySelector(".list-item:not(.dim)"));
    const extra = app.addList("work");
    await wait(50);
    check("adding a list unfolds its category", !app.collapsedCats["work"] && !!document.querySelector(`[data-list="${extra.id}"]`));
    app.deleteNode(extra.id, false);
    app.foldAllCategories(false);
    app.openList(list.id);

    // board cards open in a pop-up (the default); "Open full page" makes it the main view
    const click = (el: Element | null) => {
      const r = el!.getBoundingClientRect();
      const o = { bubbles: true, button: 0, clientX: r.left + 8, clientY: r.top + 8 };
      el!.dispatchEvent(new PointerEvent("pointerdown", o));
      window.dispatchEvent(new PointerEvent("pointerup", o));
    };
    const peekEl = () => document.querySelector<HTMLElement>("[data-peek]");
    const view = (): string => app.view; // read fresh (TypeScript would narrow app.view after assigning it)
    app.detailsId = null;
    app.view = "board";
    await wait(80);
    check("cards open in a pop-up unless set otherwise", (app.doc.settings.cardOpens ?? "popup") === "popup");
    click(document.querySelector(`[data-card="${list.id}"]`));
    await wait(80);
    check("clicking a card opens it in a pop-up over the board", view() === "board" && peekEl()?.dataset.peek === list.id, `${view()} ${app.peekId}`);
    // drag it by its header
    const header = peekEl()?.querySelector<HTMLElement>("header");
    const grow = header?.querySelector<HTMLElement>(".grow");
    const panelBox = () => peekEl()!.closest<HTMLElement>(".panel")!.getBoundingClientRect();
    if (header && grow) {
      const before = panelBox();
      const g = grow.getBoundingClientRect();
      const x0 = g.left + g.width / 2;
      const y0 = g.top + g.height / 2;
      const o = (x: number, y: number) => ({ bubbles: true, button: 0, pointerId: 1, clientX: x, clientY: y });
      grow.dispatchEvent(new PointerEvent("pointerdown", o(x0, y0)));
      header.dispatchEvent(new PointerEvent("pointermove", o(x0 - 60, y0 + 40)));
      header.dispatchEvent(new PointerEvent("pointermove", o(x0 - 120, y0 + 80)));
      header.dispatchEvent(new PointerEvent("pointerup", o(x0 - 120, y0 + 80)));
      await wait(50);
      const after = panelBox();
      const d = `${Math.round(after.left - before.left)},${Math.round(after.top - before.top)}`;
      check("the pop-up can be dragged by its header", d === "-120,80", d);
    } else check("the pop-up can be dragged by its header", false, "no header");
    const inPeek = app.append(list.id, "Added in the pop-up")!;
    await wait(50);
    check("…where its items can be edited", !!peekEl()?.querySelector(`[data-row="${inPeek.id}"]`));
    app.detailsId = inPeek.id;
    await wait(50);
    check("…and an item's details open beside it", !!document.querySelector(".details, aside") && view() === "board" && !!peekEl());
    app.detailsId = null;
    window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape" }));
    await wait(50);
    check("Esc closes the pop-up", !app.peekId && !peekEl());
    app.deleteNode(inPeek.id, false);
    click(document.querySelector(`[data-card="${second.id}"]`));
    await wait(80);
    check("a sub-list's card opens in the pop-up too", peekEl()?.dataset.peek === second.id && !!peekEl()?.textContent?.includes("Self test list"));
    peekEl()?.querySelector<HTMLButtonElement>("button.full")?.click();
    await wait(80);
    check("Open full page makes it the main view", view() === "list" && app.listId === list.id && app.flashId === second.id && !app.peekId && !peekEl());
    app.change(() => (app.doc.settings.cardOpens = "page"));
    app.view = "board";
    await wait(80);
    click(document.querySelector(`[data-card="${list.id}"]`));
    await wait(80);
    check("set to Full page, a card click opens the list", view() === "list" && app.listId === list.id && !app.peekId);
    app.openCard(list.id, "popup");
    check("…and right-click › Open in a pop-up still works", app.peekId === list.id);
    app.view = "board";
    await wait(80);
    check("the pop-up shows when the board does", !!peekEl());
    app.view = "list";
    await wait(80);
    check("leaving the board closes it", !app.peekId);
    app.change(() => (app.doc.settings.cardOpens = "popup"));
    app.openList(list.id);

    // Backspace at the start of an item joins it to the item above (real key presses on the rows)
    const jl = app.addList("work");
    app.setText(jl.id, "Join test");
    const j1 = app.append(jl.id, "One")!;
    const j2 = app.append(jl.id, "Two")!;
    const j2kid = app.append(j2.id, "kid")!;
    app.change(() => {
      const n = app.find(j2.id)!.node;
      n.links.push(lid);
      n.kind = "bullet";
    });
    await wait(80);
    const rowText = (id: string) => document.querySelector<HTMLElement>(`[data-row="${id}"] .text`);
    const backspaceAtStart = (id: string) => {
      const t = rowText(id);
      if (!t) return;
      t.focus();
      setCaret(t, 0);
      t.dispatchEvent(new KeyboardEvent("keydown", { key: "Backspace", bubbles: true, cancelable: true }));
    };
    const kids = () => app.find(jl.id)!.node.children;
    backspaceAtStart(j1.id);
    await wait(50);
    check("Backspace at the start of the first item does nothing", app.find(j1.id)?.node.text === "One" && kids().length === 2);
    backspaceAtStart(j2.id);
    await wait(80);
    const one = () => app.find(j1.id)?.node;
    check("Backspace at the start of an item joins it to the one above", !app.find(j2.id) && one()?.text === "OneTwo" && kids().length === 1, one()?.text);
    check("…its marker goes and the item above keeps its own", one()?.kind === "task");
    check("…its children and links come along", app.find(j2kid.id)?.parent?.id === j1.id && !!one()?.links.includes(lid));
    const t1 = rowText(j1.id);
    check("…and the caret sits where they join", !!t1 && document.activeElement === t1 && caretOffset(t1) === 3, `${document.activeElement?.className} ${t1 ? caretOffset(t1) : "-"}`);
    backspaceAtStart(j2kid.id);
    await wait(80);
    check("a first child joins the item it's under", !app.find(j2kid.id) && one()?.text === "OneTwokid" && one()?.children.length === 0, one()?.text);
    app.undo();
    check("Undo splits them again", app.find(j2kid.id)?.parent?.id === j1.id && one()?.text === "OneTwo");
    app.deleteNode(jl.id, false);
    app.openList(list.id);

    // categories: "New category" at the bottom, reordering
    check("the sidebar's bottom button adds a category", !!document.querySelector(".new-list")?.textContent?.includes("New category"));
    const third = app.addCategory("Third")!;
    check("a new category goes last", app.doc.categories[app.doc.categories.length - 1].id === third.id);
    app.moveCategory(third.id, "work", "before");
    await wait(50);
    const order = Array.from(document.querySelectorAll<HTMLElement>("[data-cat-block]")).map((e) => e.dataset.catBlock);
    check("moving a category reorders the sidebar", order[0] === third.id && order[1] === "work", JSON.stringify(order));
    check("an empty category says so", !!document.querySelector(`[data-cat-block="${third.id}"] .empty-cat`));
    app.deleteCategory(third.id);

    // a card in its own window: edits travel both ways; "Back into the main window" docks it as a pop-up here
    const texts = () => {
      const out: string[] = [];
      const f = app.find(list.id);
      if (f) walk(f.node, (n) => out.push(n.text));
      return out;
    };
    app.view = "board";
    await app.popOut(list.id, { w: 640, h: 520 });
    check("a card opens in its own window", (await invoke<string[]>("selftest_windows")).includes(`card-${list.id}`));
    check("…what's done there shows here", await until(() => texts().includes("Added in its own window"), 20000));
    app.append(list.id, "Ping from the main window");
    await app.flush();
    check("…and what's done here shows there", await until(() => texts().includes("Pong from the card window"), 20000));
    check("Back into the main window shows it as a pop-up here", await until(() => app.peekId === list.id, 10000), `${app.peekId}`);
    let closed = false;
    for (let i = 0; i < 30 && !closed; i++) {
      closed = !(await invoke<string[]>("selftest_windows")).includes(`card-${list.id}`);
      if (!closed) await wait(150);
    }
    check("…and closes its window", closed);
    app.closePeek();

    app.addColumn("Waiting");
    const w = app.doc.columns.find((x) => x.name === "Waiting")!;
    app.deleteColumn(w.id);
    check("an empty column can be deleted", !app.doc.columns.some((x) => x.name === "Waiting"));
    await app.flush();
  } catch (e) {
    check(`no exceptions`, false, String(e));
  }
  await invoke("selftest_done", { report: log.join("\n"), ok });
}

/** Runs in a card window during the self-test: an edit made here must reach the main window and one made
 *  there must arrive here; then the card goes back into the main window, which closes this one. */
export async function cardSelftest(id: string) {
  app.append(id, "Added in its own window");
  await app.flush();
  const ping = () => {
    let hit: string | null = null;
    const f = app.find(id);
    if (f) walk(f.node, (n) => n.text === "Ping from the main window" && (hit = n.id));
    return hit as string | null;
  };
  if (!(await until(() => !!ping(), 20000))) return;
  app.setText(ping()!, "Pong from the card window");
  await app.flush();
  await wait(300);
  await app.cardToMain("dock");
}
