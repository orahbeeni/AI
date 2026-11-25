import { open } from "@tauri-apps/plugin-dialog";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { app } from "./state.svelte";
import type { Link, MenuItem } from "./types";

export function linkIcon(l: Link | undefined): string {
  if (!l) return "link";
  if (l.kind === "folder") return "folder";
  if (l.kind === "url") return l.url?.startsWith("mailto:") ? "mail" : "globe";
  return "file";
}

export async function chooseFileFor(l: Link) {
  const picked = await open({ multiple: false, directory: l.kind === "folder", title: `Choose the new location of ${l.name}` });
  if (typeof picked === "string") await app.relink(l.id, picked);
}

export async function copyText(text: string, what = "Copied") {
  try {
    await writeText(text);
    app.say(what);
  } catch (e) {
    app.say(`Couldn't copy: ${e}`, "error");
  }
}

/** The right-click menu for a link. `nodeId` is the item it sits on (for "Remove from this item"). */
export function linkMenu(l: Link, nodeId?: string): MenuItem[] {
  const missing = l.status !== "ok";
  const remove: MenuItem = nodeId
    ? { label: "Remove link", icon: "trash", danger: true, run: () => app.detach(nodeId, l.id) }
    : { label: "Remove link everywhere", icon: "trash", danger: true, run: () => app.removeLinkEverywhere(l.id) };
  if (l.kind === "url") {
    return [
      { label: "Open", icon: "external", shortcut: "Enter", run: () => app.openLink(l.id) },
      { label: "Copy link", icon: "copy", run: () => copyText(l.url ?? "", "Link copied") },
      { separator: true, label: "" },
      remove,
    ];
  }
  if (missing) {
    return [
      { label: l.status === "searching" ? "Looking for it…" : "Can't be found", icon: "warning", disabled: true },
      { label: "Choose its new location…", icon: "folder", run: () => chooseFileFor(l) },
      { label: "See what was found", icon: "search", run: () => (app.view = "links") },
      { separator: true, label: "" },
      remove,
    ];
  }
  return [
    { label: "Open", icon: "external", shortcut: "Enter", run: () => app.openLink(l.id) },
    { label: "Show in folder", icon: "folder", run: () => app.revealLink(l.id) },
    { label: "Copy path", icon: "copy", run: () => copyText(l.path ?? "", "Path copied") },
    { separator: true, label: "" },
    {
      label: "More…",
      icon: "more",
      submenu: [{ label: "Link to another file…", icon: "refresh", run: () => chooseFileFor(l) }],
    },
    remove,
  ];
}
