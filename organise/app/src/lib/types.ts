export type Kind = "task" | "bullet";

export interface Node {
  id: string;
  text: string;
  kind: Kind;
  done: boolean;
  links: string[];
  notes: string;
  children: Node[];
  category?: string | null;
}

export interface Category {
  id: string;
  name: string;
  color: string;
  /** hidden from the sidebar and the board; its lists are kept */
  archived?: boolean;
}

export interface Column {
  id: string;
  name: string;
  color?: string | null;
  cards: string[];
}

export interface Settings {
  watched: string[];
  skipHidden: boolean;
  autoSearch: boolean;
  /** what clicking a board card does (missing means "popup") */
  cardOpens?: "popup" | "page";
}

export interface Doc {
  categories: Category[];
  columns: Column[];
  lists: Node[];
  settings: Settings;
}

export interface Candidate {
  path: string;
  reason: string;
}

export interface Link {
  id: string;
  kind: "file" | "folder" | "url";
  name: string;
  url?: string;
  size?: number;
  mtime?: number;
  fingerprint?: string;
  path?: string;
  inode?: number;
  dev?: number;
  status: "ok" | "searching" | "missing";
  candidates?: Candidate[];
}

export interface MoveRecord {
  linkId: string;
  name: string;
  from: string;
  to: string;
  at: number;
  how: string;
}

export interface LoadResult {
  doc: Doc;
  links: Link[];
  moves: MoveRecord[];
  dataDir: string;
  host: string;
  home: string;
  startView: string | null;
  selftestFile: string | null;
}

export interface TrackerStats {
  watches: number;
  budget: number;
  limited: boolean;
  error: string | null;
}

export type View = "list" | "board" | "links" | "settings";

export interface MenuItem {
  label: string;
  icon?: string;
  hint?: string;
  shortcut?: string;
  disabled?: boolean;
  danger?: boolean;
  color?: string;
  checked?: boolean;
  run?: () => void;
  submenu?: MenuItem[];
  separator?: boolean;
}
