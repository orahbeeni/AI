//! One list = one Markdown file. Readable in any editor and in Obsidian:
//!
//! ```text
//! ---
//! id: Lk3x9a2
//! title: Q4 budget review
//! category: work
//! ---
//!
//! - [x] Collect department forecasts [Forecasts_Q4.xlsx](link:f19c2aa) ^i01abcd
//!   - [ ] Child item ^i02abcd
//!     a note line under the child
//! - A bullet (no checkbox) ^i03abcd
//! ```
//!
//! Every item ends with ` ^id` (Obsidian's block-id syntax). Links appear as `[name](link:id)`.
//! Items typed by hand without an id get one the next time the file is saved.

use crate::model::{new_id, ItemKind, Node};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default)]
struct Front {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    title: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    category: Option<String>,
}

/// Parses a list file. `fallback_title` is used when the file has no title (e.g. the file name).
/// Returns the list and whether it needs rewriting (ids were missing).
pub fn parse_list(text: &str, fallback_title: &str) -> (Node, bool) {
    let text = text.replace("\r\n", "\n");
    let mut dirty = false;
    let (front, body) = split_front(&text);
    let front: Front = front.and_then(|f| serde_yaml::from_str(f).ok()).unwrap_or_default();
    let mut list = Node {
        id: front.id.clone().unwrap_or_else(|| {
            dirty = true;
            new_id("L")
        }),
        text: front.title.clone().unwrap_or_else(|| fallback_title.to_string()),
        category: front.category.clone(),
        ..Default::default()
    };
    if front.title.is_none() {
        dirty = true;
    }

    let mut flat: Vec<(usize, Node)> = vec![];
    let mut list_notes: Vec<String> = vec![];
    for line in body.lines() {
        let trimmed = line.trim_start();
        if trimmed.is_empty() {
            continue;
        }
        let indent: usize = line[..line.len() - trimmed.len()].chars().map(|c| if c == '\t' { 4 } else { 1 }).sum();
        let item = trimmed.strip_prefix("- ").or_else(|| trimmed.strip_prefix("* ")).or_else(|| trimmed.strip_prefix("+ "));
        match item {
            Some(rest) => {
                let (node, had_id) = parse_item(rest);
                if !had_id {
                    dirty = true;
                }
                flat.push((indent, node));
            }
            None => match flat.last_mut() {
                Some((_, n)) => {
                    if !n.notes.is_empty() {
                        n.notes.push('\n');
                    }
                    n.notes.push_str(trimmed.trim_end());
                }
                None => list_notes.push(trimmed.trim_end().to_string()),
            },
        }
    }
    list.notes = list_notes.join("\n");
    let mut iter = flat.into_iter().peekable();
    list.children = take_children(&mut iter, None);
    (list, dirty)
}

fn take_children(iter: &mut std::iter::Peekable<std::vec::IntoIter<(usize, Node)>>, parent: Option<usize>) -> Vec<Node> {
    let mut out = vec![];
    while let Some((ind, _)) = iter.peek() {
        if let Some(p) = parent {
            if *ind <= p {
                break;
            }
        }
        let (ind, mut node) = iter.next().unwrap();
        node.children = take_children(iter, Some(ind));
        out.push(node);
    }
    out
}

fn split_front(text: &str) -> (Option<&str>, &str) {
    if let Some(rest) = text.strip_prefix("---\n") {
        if let Some(end) = rest.find("\n---\n").map(|i| (i, i + 5)).or_else(|| rest.strip_suffix("\n---").map(|r| (r.len(), rest.len()))) {
            return (Some(&rest[..end.0]), &rest[end.1..]);
        }
    }
    (None, text)
}

/// Parses the part of an item line after "- ". Returns the node and whether it had an id.
fn parse_item(rest: &str) -> (Node, bool) {
    let mut node = Node::default();
    let mut s = rest.trim_end().to_string();
    // checkbox
    if let Some(r) = s.strip_prefix("[ ] ").or_else(|| (s == "[ ]").then_some("")) {
        node.kind = ItemKind::Task;
        s = r.to_string();
    } else if let Some(r) = s.strip_prefix("[x] ").or_else(|| s.strip_prefix("[X] ")).or_else(|| (s == "[x]" || s == "[X]").then_some("")) {
        node.kind = ItemKind::Task;
        node.done = true;
        s = r.to_string();
    } else {
        node.kind = ItemKind::Bullet;
    }
    // block id
    let mut had_id = false;
    if let Some(pos) = s.rfind(" ^").or_else(|| s.starts_with('^').then_some(0)) {
        let start = if pos == 0 && s.starts_with('^') { 1 } else { pos + 2 };
        let id = &s[start..];
        if !id.is_empty() && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
            node.id = id.to_string();
            had_id = true;
            s.truncate(pos);
        }
    }
    if !had_id {
        node.id = new_id("i");
    }
    // trailing links: [label](link:id)
    let mut links = vec![];
    loop {
        let t = s.trim_end();
        if !t.ends_with(')') {
            break;
        }
        let Some(mid) = t.rfind("](link:") else { break };
        let id = &t[mid + 7..t.len() - 1];
        if id.is_empty() || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
            break;
        }
        // find the unescaped '[' that opens the label
        let bytes = t.as_bytes();
        let mut open = None;
        let mut i = mid;
        while i > 0 {
            i -= 1;
            if bytes[i] == b'[' && (i == 0 || bytes[i - 1] != b'\\') {
                open = Some(i);
                break;
            }
        }
        let Some(open) = open else { break };
        links.push(id.to_string());
        s = t[..open].to_string();
    }
    links.reverse();
    node.links = links;
    node.text = s.trim().to_string();
    (node, had_id)
}

fn escape_label(s: &str) -> String {
    s.replace('\\', "\\\\").replace('[', "\\[").replace(']', "\\]")
}

/// Writes a list file. `link_name` gives the display name for a link id.
pub fn write_list(list: &Node, link_name: &dyn Fn(&str) -> Option<String>) -> String {
    let front = Front { id: Some(list.id.clone()), title: Some(list.text.clone()), category: list.category.clone() };
    let mut out = String::from("---\n");
    out.push_str(&serde_yaml::to_string(&front).unwrap_or_default());
    out.push_str("---\n\n");
    if !list.notes.trim().is_empty() {
        for l in list.notes.lines().filter(|l| !l.trim().is_empty()) {
            out.push_str(l.trim());
            out.push('\n');
        }
        out.push('\n');
    }
    for c in &list.children {
        write_item(&mut out, c, 0, link_name);
    }
    out
}

fn write_item(out: &mut String, n: &Node, depth: usize, link_name: &dyn Fn(&str) -> Option<String>) {
    let pad = "  ".repeat(depth);
    out.push_str(&pad);
    out.push_str("- ");
    if n.kind == ItemKind::Task {
        out.push_str(if n.done { "[x] " } else { "[ ] " });
    }
    let text = n.text.replace(['\n', '\r'], " ");
    out.push_str(text.trim());
    for l in &n.links {
        let name = link_name(l).unwrap_or_else(|| "link".into());
        out.push_str(&format!(" [{}](link:{})", escape_label(&name), l));
    }
    out.push_str(" ^");
    out.push_str(&n.id);
    out.push('\n');
    for l in n.notes.lines().filter(|l| !l.trim().is_empty()) {
        out.push_str(&pad);
        out.push_str("  ");
        out.push_str(l.trim());
        out.push('\n');
    }
    for c in &n.children {
        write_item(out, c, depth + 1, link_name);
    }
}

/// A file name for a list: lower-case words joined by dashes.
pub fn slug(title: &str) -> String {
    let mut s = String::new();
    let mut dash = false;
    for c in title.chars().flat_map(|c| c.to_lowercase()) {
        if c.is_alphanumeric() {
            s.push(c);
            dash = false;
        } else if !dash && !s.is_empty() {
            s.push('-');
            dash = true;
        }
    }
    let s = s.trim_end_matches('-').chars().take(60).collect::<String>();
    if s.is_empty() {
        "list".into()
    } else {
        s
    }
}
