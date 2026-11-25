use organise_core::engine::Engine;
use organise_core::links::{Link, LinkStatus};
use organise_core::markdown::{parse_list, write_list};
use organise_core::model::{CardOpens, Doc, ItemKind, Node};
use organise_core::scan::{search, Found, Target};
use organise_core::store::Store;
use organise_core::tracker::{FsEvent, Targets, Tracker};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};

fn node(id: &str, text: &str, kind: ItemKind, done: bool, links: &[&str], children: Vec<Node>) -> Node {
    Node { id: id.into(), text: text.into(), kind, done, links: links.iter().map(|s| s.to_string()).collect(), children, ..Default::default() }
}

#[test]
fn markdown_round_trip() {
    let mut list = node("Labc", "Q4 budget review", ItemKind::Task, false, &[], vec![
        node("i1", "Collect forecasts", ItemKind::Task, true, &["f1", "f2"], vec![
            node("i2", "Child with [brackets] and ^caret", ItemKind::Bullet, false, &[], vec![]),
        ]),
        node("i3", "", ItemKind::Task, false, &[], vec![]),
    ]);
    list.category = Some("work".into());
    list.children[0].notes = "line one\nline two".into();
    let names = |id: &str| Some(match id { "f1" => "Forecasts [Q4].xlsx".to_string(), _ => "HR.csv".to_string() });
    let text = write_list(&list, &names);
    assert!(text.contains("- [x] Collect forecasts [Forecasts \\[Q4\\].xlsx](link:f1) [HR.csv](link:f2) ^i1\n"), "{text}");
    let (back, dirty) = parse_list(&text, "x");
    assert!(!dirty);
    assert_eq!(back, list, "\n{text}");
}

#[test]
fn markdown_hand_written_and_obsidian_tabs() {
    let text = "# no front matter\n- [ ] top\n\t- [x] child with tab\n\t\t- deeper bullet\n- second\n  a note\n";
    let (l, dirty) = parse_list(text, "my list");
    assert!(dirty, "ids were missing");
    assert_eq!(l.text, "my list");
    assert_eq!(l.notes, "# no front matter");
    assert_eq!(l.children.len(), 2);
    assert_eq!(l.children[0].children[0].text, "child with tab");
    assert!(l.children[0].children[0].done);
    assert_eq!(l.children[0].children[0].children[0].kind, ItemKind::Bullet);
    assert_eq!(l.children[1].notes, "a note");
}

#[test]
fn store_save_load_rename_delete() {
    let dir = tempfile::tempdir().unwrap();
    let data = dir.path().join("Organise");
    let mut st = Store::open(&data).unwrap();
    assert_eq!(st.doc.lists.len(), 1, "starter list");
    let f = dir.path().join("report.pdf");
    std::fs::write(&f, b"hello").unwrap();
    let link = st.add_link(Link::from_path(&f).unwrap());
    st.doc.lists[0].children[0].links.push(link.id.clone());
    let mut second = node("Lsecond", "Second list", ItemKind::Task, false, &[], vec![node("ix", "x", ItemKind::Task, false, &[], vec![])]);
    second.category = Some("work".into());
    st.doc.lists.push(second);
    st.save_doc().unwrap();
    st.save_links().unwrap();
    assert!(data.join("lists/second-list.md").exists());

    assert_eq!(st.doc.settings.card_opens, CardOpens::Popup, "cards open in a pop-up unless set otherwise");
    st.doc.categories[0].archived = true;
    st.doc.settings.card_opens = CardOpens::Page;
    st.save_doc().unwrap();
    let st2 = Store::open(&data).unwrap();
    assert_eq!(st2.doc, st.doc);
    assert!(st2.doc.categories[0].archived, "archived flag survives a reload");
    let ws = std::fs::read_to_string(data.join("workspace.yaml")).unwrap();
    assert!(ws.contains("archived: true"));
    assert!(ws.contains("cardOpens: page"), "card setting is saved: {ws}");
    st.doc.categories[0].archived = false;
    assert_eq!(st2.links.get(&link.id).unwrap().path, link.path);
    assert_eq!(st2.links.get(&link.id).unwrap().inode, link.inode);

    // rename a list -> its file is renamed; delete a list -> file goes to .trash
    st.doc.lists[1].text = "Renamed list".into();
    st.save_doc().unwrap();
    assert!(data.join("lists/renamed-list.md").exists());
    assert!(!data.join("lists/second-list.md").exists());
    st.doc.lists.remove(1);
    st.save_doc().unwrap();
    assert!(!data.join("lists/renamed-list.md").exists());
    assert_eq!(std::fs::read_dir(data.join(".trash")).unwrap().count(), 1);

    // backup and restore
    let zip = dir.path().join("backup.zip");
    organise_core::backup::export(&data, &zip).unwrap();
    st.doc.lists[0].text = "Changed after backup".into();
    st.save_doc().unwrap();
    organise_core::backup::restore(&data, &zip).unwrap();
    let st3 = Store::open(&data).unwrap();
    assert_eq!(st3.doc.lists[0].text, "Welcome to Organise");
}

#[test]
fn two_lists_with_the_same_title_get_two_files() {
    let dir = tempfile::tempdir().unwrap();
    let mut st = Store::open(dir.path()).unwrap();
    st.doc.lists.push(node("La", "Same", ItemKind::Task, false, &[], vec![]));
    st.doc.lists.push(node("Lb", "Same", ItemKind::Task, false, &[], vec![]));
    st.save_doc().unwrap();
    let st2 = Store::open(dir.path()).unwrap();
    assert_eq!(st2.doc.lists.len(), 3);
}

fn links_with(paths: &[&Path]) -> BTreeMap<String, Link> {
    paths.iter().map(|p| { let l = Link::from_path(p).unwrap(); (l.id.clone(), l) }).collect()
}

/// Feeds tracker events into the engine until `done` says so, or 5 s pass.
fn drive(engine: &mut Engine, links: &mut BTreeMap<String, Link>, rx: &std::sync::mpsc::Receiver<FsEvent>, done: impl Fn(&BTreeMap<String, Link>) -> bool) -> Vec<String> {
    let start = Instant::now();
    let mut search = vec![];
    while start.elapsed() < Duration::from_secs(5) {
        while let Ok(ev) = rx.try_recv() {
            let o = engine.on_event(&ev, links);
            search.extend(o.search);
        }
        let o = engine.tick(links);
        search.extend(o.search);
        if done(links) {
            return search;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    search
}

#[test]
fn watcher_follows_rename_move_and_ignores_safe_save() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let a = root.join("a");
    let b = root.join("a/sub");
    std::fs::create_dir_all(&b).unwrap();
    let f = a.join("report.txt");
    let g = a.join("notes.txt");
    std::fs::write(&f, "v1").unwrap();
    std::fs::write(&g, "notes").unwrap();
    let mut links = links_with(&[&f, &g]);
    let fid = links.values().find(|l| l.name == "report.txt").unwrap().id.clone();
    let gid = links.values().find(|l| l.name == "notes.txt").unwrap().id.clone();

    let (tx, rx) = channel();
    let tracker = Tracker::start(move |e| { let _ = tx.send(e); });
    tracker.set_targets(Targets { dirs: vec![], roots: vec![root.clone()], skip_hidden: true });
    std::thread::sleep(Duration::from_millis(300));
    let mut engine = Engine::default();

    // 1. rename + move into a sub-folder
    let f2 = b.join("report-final.txt");
    std::fs::rename(&f, &f2).unwrap();
    drive(&mut engine, &mut links, &rx, |l| l[&fid].path.as_deref() == Some(f2.to_str().unwrap()));
    assert_eq!(links[&fid].path.as_deref(), Some(f2.to_str().unwrap()), "rename+move followed");

    // 2. "safe save": rename the old file to a backup name, write a new one under the same name
    std::fs::rename(&g, a.join("notes.txt~")).unwrap();
    std::fs::write(&g, "notes v2").unwrap();
    std::thread::sleep(organise_core::engine::DELAY + Duration::from_millis(300));
    drive(&mut engine, &mut links, &rx, |_| false);
    assert_eq!(links[&gid].path.as_deref(), Some(g.to_str().unwrap()), "safe save keeps the link on the same name");
    assert_eq!(links[&gid].status, LinkStatus::Ok);

    // 3. folder move: the link inside follows
    let moved = root.join("moved");
    std::fs::rename(&b, &moved).unwrap();
    let f3 = moved.join("report-final.txt");
    drive(&mut engine, &mut links, &rx, |l| l[&fid].path.as_deref() == Some(f3.to_str().unwrap()));
    assert_eq!(links[&fid].path.as_deref(), Some(f3.to_str().unwrap()), "folder move followed");
}

#[test]
fn move_out_of_watched_folders_is_found_by_inode_search() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    let watched = root.join("watched");
    let elsewhere = root.join("x/y/z");
    std::fs::create_dir_all(&watched).unwrap();
    std::fs::create_dir_all(&elsewhere).unwrap();
    let f = watched.join("plan.ods");
    std::fs::write(&f, "plan").unwrap();
    let mut links = links_with(&[&f]);
    let id = links.keys().next().unwrap().clone();

    let (tx, rx) = channel();
    let tracker = Tracker::start(move |e| { let _ = tx.send(e); });
    tracker.set_targets(Targets { dirs: vec![watched.clone()], roots: vec![], skip_hidden: true });
    std::thread::sleep(Duration::from_millis(300));
    let mut engine = Engine::default();
    let to = elsewhere.join("plan renamed.ods");
    std::fs::rename(&f, &to).unwrap();
    let search_ids = drive(&mut engine, &mut links, &rx, |l| l.values().next().unwrap().status == LinkStatus::Searching);
    assert_eq!(search_ids, vec![id.clone()], "a move out of the watched folder asks for a search");

    let targets: Vec<Target> = search_ids.iter().map(|i| Target::from_link(&links[i])).collect();
    let found = search(&targets, &[root.clone()], true, &AtomicBool::new(false));
    assert_eq!(found[&id], Found::Exact { path: to.clone(), how: "inode" });
    let o = engine.apply_found(found, &mut links);
    assert_eq!(o.moves.len(), 1);
    assert_eq!(links[&id].path.as_deref(), Some(to.to_str().unwrap()));
    assert_eq!(links[&id].status, LinkStatus::Ok);
}

#[test]
fn copy_and_delete_is_found_by_content_and_startup_verify() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().canonicalize().unwrap();
    std::fs::create_dir_all(root.join("old")).unwrap();
    std::fs::create_dir_all(root.join("archive/2026")).unwrap();
    let f = root.join("old/Budget.pptx");
    let content: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
    std::fs::write(&f, &content).unwrap();
    let mut links = links_with(&[&f]);
    let id = links.keys().next().unwrap().clone();
    // a cross-disk move is a copy + delete: new inode, same content
    std::fs::write(root.join("archive/2026/Budget.pptx"), &content).unwrap();
    std::fs::remove_file(&f).unwrap();
    // a decoy with the same name but different content
    std::fs::write(root.join("Budget.pptx"), b"something else").unwrap();

    let mut engine = Engine::default();
    let o = engine.verify(&mut links);
    assert_eq!(o.search, vec![id.clone()]);
    let found = search(&[Target::from_link(&links[&id])], &[root.clone()], true, &AtomicBool::new(false));
    assert_eq!(found[&id], Found::Exact { path: root.join("archive/2026/Budget.pptx"), how: "content" });

    // nothing similar anywhere -> missing, with the decoy offered as a candidate
    std::fs::remove_file(root.join("archive/2026/Budget.pptx")).unwrap();
    let found = search(&[Target::from_link(&links[&id])], &[root.clone()], true, &AtomicBool::new(false));
    engine.apply_found(found, &mut links);
    assert_eq!(links[&id].status, LinkStatus::Missing);
    assert_eq!(links[&id].candidates.len(), 1);
    assert_eq!(links[&id].candidates[0].reason, "Same name");
}

#[test]
fn starter_doc_is_consistent() {
    let d = Doc::starter();
    assert_eq!(d.columns.len(), 5);
    assert!(d.node_ids().contains(&d.columns[0].cards[0]));
}

#[test]
fn changing_watch_targets_right_after_a_move_keeps_the_move() {
    // Regression: re-targeting used to drop events the kernel had already queued.
    for round in 0..5 {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path().canonicalize().unwrap();
        let a = root.join("a");
        let b = root.join("b");
        std::fs::create_dir_all(&a).unwrap();
        std::fs::create_dir_all(&b).unwrap();
        let f = a.join("doc.txt");
        std::fs::write(&f, "x").unwrap();
        let mut links = links_with(&[&f]);
        let id = links.keys().next().unwrap().clone();
        let (tx, rx) = channel();
        let tracker = Tracker::start(move |e| { let _ = tx.send(e); });
        tracker.set_targets(Targets { dirs: vec![a.clone()], roots: vec![], skip_hidden: true });
        std::thread::sleep(Duration::from_millis(200));
        let f2 = a.join("doc renamed.txt");
        std::fs::rename(&f, &f2).unwrap();
        tracker.set_targets(Targets { dirs: vec![a.clone(), b.clone()], roots: vec![], skip_hidden: true });
        let mut engine = Engine::default();
        drive(&mut engine, &mut links, &rx, |l| l[&id].path.as_deref() == Some(f2.to_str().unwrap()));
        assert_eq!(links[&id].path.as_deref(), Some(f2.to_str().unwrap()), "round {round}: move lost after re-targeting");
    }
}
