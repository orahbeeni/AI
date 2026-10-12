//! Render a flat list of paths as a collapsible directory tree.

use eframe::egui;

pub enum Node<'a> {
    Dir(String, Vec<Node<'a>>),
    File(&'a str, usize),
}

/// Group `(path, id)` items by directory.
///
/// * `compact` merges single-child directory chains into one node ("a/b/c").
/// * `dirs_first` lists directories before files instead of sorting them
///   together by name.
pub fn build<'a>(items: &[(&'a str, usize)], compact: bool, dirs_first: bool) -> Vec<Node<'a>> {
    let mut sorted = items.to_vec();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    let mut nodes: Vec<Node<'a>> = Vec::new();
    let mut i = 0;
    while i < sorted.len() {
        let (path, id) = sorted[i];
        match path.split_once('/') {
            None => {
                nodes.push(Node::File(path, id));
                i += 1;
            }
            Some((dir, _)) => {
                let mut group = Vec::new();
                while i < sorted.len() {
                    match sorted[i].0.split_once('/') {
                        Some((d, rest)) if d == dir => group.push((rest, sorted[i].1)),
                        _ => break,
                    }
                    i += 1;
                }
                let mut name = dir.to_owned();
                let mut children = build(&group, compact, dirs_first);
                while compact && matches!(children.as_slice(), [Node::Dir(..)]) {
                    let Some(Node::Dir(n, c)) = children.pop() else { break };
                    name = format!("{name}/{n}");
                    children = c;
                }
                nodes.push(Node::Dir(name, children));
            }
        }
    }
    if dirs_first {
        // Stable: keeps name order within each group.
        nodes.sort_by_key(|n| matches!(n, Node::File(..)));
    }
    nodes
}

fn render(
    ui: &mut egui::Ui,
    salt: &str,
    prefix: &str,
    nodes: &[Node],
    file: &mut dyn FnMut(&mut egui::Ui, usize, &str),
) {
    for n in nodes {
        match n {
            Node::File(name, id) => file(ui, *id, name),
            Node::Dir(name, children) => {
                let full = format!("{prefix}{name}/");
                egui::CollapsingHeader::new(name)
                    .id_salt((salt, &full))
                    .default_open(true)
                    .show(ui, |ui| render(ui, salt, &full, children, file));
            }
        }
    }
}

/// Draw `items` as a tree. `file` draws one file row given its id and bare name.
/// `salt` keeps collapse state separate between lists.
pub fn show(
    ui: &mut egui::Ui,
    salt: &str,
    items: &[(&str, usize)],
    file: &mut dyn FnMut(&mut egui::Ui, usize, &str),
) {
    let nodes = build(items, true, true);
    render(ui, salt, "", &nodes, file);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn flat(nodes: &[Node], out: &mut Vec<String>, depth: usize) {
        for n in nodes {
            match n {
                Node::File(name, id) => out.push(format!("{}{name}#{id}", " ".repeat(depth))),
                Node::Dir(name, c) => {
                    out.push(format!("{}{name}/", " ".repeat(depth)));
                    flat(c, out, depth + 1);
                }
            }
        }
    }

    #[test]
    fn mixed_order_without_compaction() {
        let items = [("hotfix/x", 0), ("master", 1), ("release/a/b", 2), ("ECOMD-1", 3)];
        let mut out = Vec::new();
        flat(&build(&items, false, false), &mut out, 0);
        assert_eq!(
            out,
            vec!["ECOMD-1#3", "hotfix/", " x#0", "master#1", "release/", " a/", "  b#2"]
        );
    }

    #[test]
    fn groups_and_compacts() {
        let items = [("src/a/x.rs", 0), ("src/a/y.rs", 1), ("src/b/z.rs", 2), ("README", 3), ("deep/er/est/f", 4)];
        let mut out = Vec::new();
        flat(&build(&items, true, true), &mut out, 0);
        assert_eq!(
            out,
            vec![
                "deep/er/est/",
                " f#4",
                "src/",
                " a/",
                "  x.rs#0",
                "  y.rs#1",
                " b/",
                "  z.rs#2",
                "README#3",
            ]
        );
    }
}
