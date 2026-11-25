//! The document: categories, board columns, lists (trees of items) and settings.
//! Links are kept separately in a registry (see `links.rs`); items refer to them by id.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum ItemKind {
    #[default]
    Task,
    Bullet,
}

/// A list (top level) or an item inside one. Any item can have children.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Node {
    pub id: String,
    pub text: String,
    #[serde(default)]
    pub kind: ItemKind,
    #[serde(default)]
    pub done: bool,
    #[serde(default)]
    pub links: Vec<String>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub children: Vec<Node>,
    /// Only used on lists (top-level nodes). Items inherit their list's category.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Category {
    pub id: String,
    pub name: String,
    pub color: String,
    /// Hidden from the sidebar and the board until unarchived. Its lists are kept as they are.
    #[serde(default, skip_serializing_if = "is_false")]
    pub archived: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Column {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Ids of the lists (or sub-lists) shown as cards, top to bottom.
    #[serde(default)]
    pub cards: Vec<String>,
}

fn yes() -> bool {
    true
}

/// What clicking a card on the board does.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "lowercase")]
pub enum CardOpens {
    /// A pop-up over the board, with a button to open it as the main view.
    #[default]
    Popup,
    /// Straight to the list, full size.
    Page,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    /// Folders watched recursively (moves inside them are seen instantly).
    /// The folders that hold linked files are always watched.
    #[serde(default)]
    pub watched: Vec<String>,
    #[serde(default = "yes")]
    pub skip_hidden: bool,
    /// Search for a missing file automatically.
    #[serde(default = "yes")]
    pub auto_search: bool,
    #[serde(default)]
    pub card_opens: CardOpens,
}

impl Default for Settings {
    fn default() -> Self {
        Settings { watched: vec![], skip_hidden: true, auto_search: true, card_opens: CardOpens::Popup }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub struct Doc {
    pub categories: Vec<Category>,
    pub columns: Vec<Column>,
    pub lists: Vec<Node>,
    #[serde(default)]
    pub settings: Settings,
}

pub fn new_id(prefix: &str) -> String {
    use rand::Rng;
    const CHARS: &[u8] = b"abcdefghijkmnopqrstuvwxyz23456789";
    let mut rng = rand::thread_rng();
    let s: String = (0..7).map(|_| CHARS[rng.gen_range(0..CHARS.len())] as char).collect();
    format!("{prefix}{s}")
}

impl Node {
    pub fn new(text: &str) -> Node {
        Node { id: new_id("i"), text: text.to_string(), ..Default::default() }
    }

    pub fn walk<'a>(&'a self, f: &mut dyn FnMut(&'a Node)) {
        f(self);
        for c in &self.children {
            c.walk(f);
        }
    }
}

impl Doc {
    /// A fresh workspace: default categories and columns plus a short welcome list.
    pub fn starter() -> Doc {
        let cat = |id: &str, name: &str, color: &str| Category { id: id.into(), name: name.into(), color: color.into(), archived: false };
        let col = |id: &str, name: &str| Column { id: id.into(), name: name.into(), color: None, cards: vec![] };
        let mut welcome = Node { id: new_id("L"), text: "Welcome to Organise".into(), category: Some("personal".into()), ..Default::default() };
        let item = |t: &str, kind: ItemKind, children: Vec<Node>| Node { kind, children, ..Node::new(t) };
        welcome.children = vec![
            item("Drag a file or folder from the file manager onto this item", ItemKind::Task, vec![
                item("Only a link is saved. The file stays where it is.", ItemKind::Bullet, vec![]),
                item("Move or rename the file later and the link follows it.", ItemKind::Bullet, vec![]),
            ]),
            item("Click a link to open it, right-click for Show in folder", ItemKind::Task, vec![]),
            item("Press Enter for a new item, Tab / Shift+Tab to nest", ItemKind::Task, vec![]),
            item("Drag the ⋮ handle to move an item, or drop it on a list in the sidebar", ItemKind::Task, vec![]),
            item("Open the Board and drag this list between columns", ItemKind::Task, vec![]),
        ];
        let mut columns = vec![
            col("todo", "To do"),
            col("doing", "In progress"),
            col("blocked", "Blocked"),
            col("review", "Under review"),
            col("done", "Done"),
        ];
        columns[0].cards.push(welcome.id.clone());
        Doc {
            categories: vec![cat("work", "Work", "#3b6fd8"), cat("personal", "Personal", "#2f9e62")],
            columns,
            lists: vec![welcome],
            settings: Settings::default(),
        }
    }

    /// Every link id referenced anywhere in the document.
    pub fn link_ids(&self) -> std::collections::HashSet<String> {
        let mut out = std::collections::HashSet::new();
        for l in &self.lists {
            l.walk(&mut |n| out.extend(n.links.iter().cloned()));
        }
        out
    }

    /// Every node id in the document.
    pub fn node_ids(&self) -> std::collections::HashSet<String> {
        let mut out = std::collections::HashSet::new();
        for l in &self.lists {
            l.walk(&mut |n| {
                out.insert(n.id.clone());
            });
        }
        out
    }
}
