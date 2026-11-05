//! A Model Context Protocol server on stdin/stdout, so AI assistants can read, design and
//! render screens. It edits the same project file the editor has open; the editor reloads it.

use crate::catalog;
use crate::model::{self, Align, Control, Project, State};
use crate::render;
use serde_json::{json, Value};
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

pub fn run(file: Option<PathBuf>) {
    let stdin = std::io::stdin();
    let mut out = std::io::stdout();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        if line.trim().is_empty() {
            continue;
        }
        let msg: Value = match serde_json::from_str(&line) {
            Ok(v) => v,
            Err(e) => {
                let r = json!({"jsonrpc": "2.0", "id": null, "error": {"code": -32700, "message": e.to_string()}});
                let _ = writeln!(out, "{r}");
                let _ = out.flush();
                continue;
            }
        };
        let path = file.clone().unwrap_or_else(model::current_file);
        if let Some(reply) = handle(&msg, &path) {
            let _ = writeln!(out, "{reply}");
            let _ = out.flush();
        }
    }
}

fn handle(msg: &Value, path: &Path) -> Option<Value> {
    let id = msg.get("id").cloned();
    let method = msg.get("method").and_then(|m| m.as_str()).unwrap_or("");
    let params = msg.get("params").cloned().unwrap_or(json!({}));
    let id = id?; // notifications get no reply
    let result = match method {
        "initialize" => Ok(json!({
            "protocolVersion": params.get("protocolVersion").and_then(|v| v.as_str()).unwrap_or("2025-06-18"),
            "capabilities": {"tools": {"listChanged": false}},
            "serverInfo": {"name": "sketchframe", "title": "Sketchframe screen designer", "version": env!("CARGO_PKG_VERSION")},
            "instructions": INSTRUCTIONS,
        })),
        "ping" => Ok(json!({})),
        "tools/list" => Ok(json!({"tools": tools()})),
        "tools/call" => {
            let name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let args = params.get("arguments").cloned().unwrap_or(json!({}));
            Ok(match call(name, &args, path) {
                Ok(content) => json!({"content": content}),
                Err(e) => json!({"content": [{"type": "text", "text": e}], "isError": true}),
            })
        }
        "resources/list" => Ok(json!({"resources": []})),
        "prompts/list" => Ok(json!({"prompts": []})),
        _ => Err(json!({"code": -32601, "message": format!("unknown method {method}")})),
    };
    Some(match result {
        Ok(r) => json!({"jsonrpc": "2.0", "id": id, "result": r}),
        Err(e) => json!({"jsonrpc": "2.0", "id": id, "error": e}),
    })
}

const INSTRUCTIONS: &str = "Sketchframe is a wireframe / mockup tool. A project has screens (each sized for a device, e.g. a phone 393x852 or a browser page 1280x800) and each screen holds controls placed at x,y with a width and height in pixels, top-left origin. Start with get_project, call list_control_types once to learn the control types and their text syntax, then create_screen and add_controls (many at once). Use render_screen to look at the result and fix overlaps. Link controls to other screens with the link field to make a clickable prototype. The person sees every change live in the app.";

fn tools() -> Value {
    let control_props = json!({
        "type": {"type": "string", "description": "Control type id from list_control_types, e.g. button, text_input, heading, list, table, bottom_nav"},
        "x": {"type": "number"}, "y": {"type": "number"},
        "w": {"type": "number", "description": "Width (default from the control type)"},
        "h": {"type": "number", "description": "Height (default from the control type)"},
        "text": {"type": "string", "description": "The control's text. Many controls read lists from it (see list_control_types)."},
        "state": {"type": "string", "enum": ["normal", "selected", "disabled", "focused"]},
        "value": {"type": "number", "description": "Slider/progress percent, star count, chosen day, current page…"},
        "fill": {"type": "string", "description": "Background colour #rrggbb"},
        "color": {"type": "string", "description": "Text/line colour #rrggbb"},
        "font_size": {"type": "number"}, "bold": {"type": "boolean"}, "italic": {"type": "boolean"}, "underline": {"type": "boolean"},
        "align": {"type": "string", "enum": ["auto", "left", "center", "right"]},
        "opacity": {"type": "number", "description": "0..1"},
        "link": {"type": "string", "description": "Screen id or name to open when clicked in present mode"},
        "component": {"type": "string", "description": "For type component: the component screen to show"},
        "name": {"type": "string", "description": "Layer name"},
        "fit": {"type": "string", "enum": ["stretch", "contain", "cover"], "description": "For type picture: how the picture sits in its box"},
        "locked": {"type": "boolean"}, "hidden": {"type": "boolean"}
    });
    let screen = json!({"type": "string", "description": "Screen id (like s1) or its name"});
    json!([
        {"name": "get_project", "description": "Overview of the open project: its screens with ids, devices, sizes and control counts.",
         "inputSchema": {"type": "object", "properties": {}}},
        {"name": "list_control_types", "description": "Every control type with its default size, default text and how its text is read; plus device sizes and icon names.",
         "inputSchema": {"type": "object", "properties": {"category": {"type": "string", "description": "Only this category (optional)"}}}},
        {"name": "get_screen", "description": "All controls of one screen as JSON (ids, types, positions, text).",
         "inputSchema": {"type": "object", "properties": {"screen": screen}, "required": ["screen"]}},
        {"name": "create_screen", "description": "Add a screen. device sets its size and frame (phone_island, android, tablet, browser, laptop, watch, tv, car, spatial, none…).",
         "inputSchema": {"type": "object", "properties": {
            "name": {"type": "string"}, "device": {"type": "string"}, "width": {"type": "number"}, "height": {"type": "number"},
            "dark": {"type": "boolean", "description": "Dark-mode wireframe"}, "is_component": {"type": "boolean", "description": "A reusable component that can be placed in other screens"},
            "background": {"type": "string", "description": "Another screen drawn underneath (a master/template)"},
            "notes": {"type": "string"}}, "required": ["name"]}},
        {"name": "update_screen", "description": "Rename or change a screen's device, size, notes, dark mode, background screen or component flag.",
         "inputSchema": {"type": "object", "properties": {
            "screen": screen, "name": {"type": "string"}, "device": {"type": "string"}, "width": {"type": "number"}, "height": {"type": "number"},
            "dark": {"type": "boolean"}, "is_component": {"type": "boolean"}, "background": {"type": "string", "description": "Screen id, or empty to remove"}, "notes": {"type": "string"}},
            "required": ["screen"]}},
        {"name": "duplicate_screen", "description": "Copy a screen with all its controls.",
         "inputSchema": {"type": "object", "properties": {"screen": screen, "name": {"type": "string"}}, "required": ["screen"]}},
        {"name": "delete_screen", "description": "Remove a screen.",
         "inputSchema": {"type": "object", "properties": {"screen": screen}, "required": ["screen"]}},
        {"name": "add_controls", "description": "Place one or more controls on a screen. Returns their ids. Later controls draw on top of earlier ones.",
         "inputSchema": {"type": "object", "properties": {"screen": screen,
            "controls": {"type": "array", "items": {"type": "object", "properties": control_props.clone(), "required": ["type", "x", "y"]}}},
            "required": ["screen", "controls"]}},
        {"name": "add_picture", "description": "Add the user's own picture (PNG, JPEG, GIF, WebP, BMP, ICO, TIFF or SVG) to a screen as a 'picture' control. PNG/GIF/WebP/SVG transparency is kept. Give a file path on this computer, or base64 data. Without width/height it keeps the picture's proportions (scaled down to fit the screen). Returns the control id.",
         "inputSchema": {"type": "object", "properties": {"screen": screen,
            "path": {"type": "string", "description": "Path of a picture file"},
            "data": {"type": "string", "description": "Base64 of the picture file, instead of a path"},
            "name": {"type": "string", "description": "File name for the picture when using data"},
            "x": {"type": "number", "description": "Left edge"}, "y": {"type": "number", "description": "Top edge"},
            "width": {"type": "number"}, "height": {"type": "number"},
            "fit": {"type": "string", "enum": ["stretch", "contain", "cover"]}}, "required": ["screen"]}},
        {"name": "update_controls", "description": "Change fields of existing controls (move, resize, retext, restyle, link…). Only the given fields change.",
         "inputSchema": {"type": "object", "properties": {"screen": screen,
            "updates": {"type": "array", "items": {"type": "object", "properties": {
                "id": {"type": "integer"}, "type": control_props["type"].clone(), "x": {"type": "number"}, "y": {"type": "number"}, "w": {"type": "number"}, "h": {"type": "number"},
                "text": {"type": "string"}, "state": control_props["state"].clone(), "value": {"type": "number"}, "fill": {"type": "string"}, "color": {"type": "string"},
                "font_size": {"type": "number"}, "bold": {"type": "boolean"}, "italic": {"type": "boolean"}, "underline": {"type": "boolean"}, "align": control_props["align"].clone(),
                "opacity": {"type": "number"}, "link": {"type": "string"}, "component": {"type": "string"}, "name": {"type": "string"}, "locked": {"type": "boolean"}, "hidden": {"type": "boolean"}},
                "required": ["id"]}}},
            "required": ["screen", "updates"]}},
        {"name": "delete_controls", "description": "Remove controls by id.",
         "inputSchema": {"type": "object", "properties": {"screen": screen, "ids": {"type": "array", "items": {"type": "integer"}}}, "required": ["screen", "ids"]}},
        {"name": "clear_screen", "description": "Remove every control from a screen.",
         "inputSchema": {"type": "object", "properties": {"screen": screen}, "required": ["screen"]}},
        {"name": "render_screen", "description": "Draw a screen and return it as a PNG picture so you can check the layout.",
         "inputSchema": {"type": "object", "properties": {"screen": screen,
            "frame": {"type": "boolean", "description": "Include the device frame (default true)"},
            "sketch": {"type": "boolean", "description": "Hand-drawn look (default true); false for clean lines"},
            "scale": {"type": "number", "description": "Picture scale, default 1"}}, "required": ["screen"]}},
        {"name": "export", "description": "Write the project to a file: png or svg (one screen), html (clickable prototype of all screens) or json (the project).",
         "inputSchema": {"type": "object", "properties": {"format": {"type": "string", "enum": ["png", "svg", "html", "json"]},
            "path": {"type": "string", "description": "Absolute file path"}, "screen": screen, "sketch": {"type": "boolean"}}, "required": ["format", "path"]}},
        {"name": "save_version", "description": "Store a named snapshot of all screens in the project's version history.",
         "inputSchema": {"type": "object", "properties": {"name": {"type": "string"}}, "required": ["name"]}}
    ])
}

fn text(s: impl Into<String>) -> Vec<Value> {
    vec![json!({"type": "text", "text": s.into()})]
}

fn load(path: &Path) -> Project {
    Project::load(path).unwrap_or_else(|_| Project::empty())
}

fn screen_arg(p: &Project, args: &Value) -> Result<usize, String> {
    let key = args.get("screen").and_then(|v| v.as_str().map(|s| s.to_string()).or_else(|| v.as_u64().map(|n| n.to_string()))).ok_or("screen is required")?;
    p.screen_idx(&key).ok_or_else(|| {
        let names: Vec<String> = p.screens.iter().map(|s| format!("{} ({})", s.id, s.name)).collect();
        format!("no screen '{key}'. Screens: {}", names.join(", "))
    })
}

fn summary(p: &Project, path: &Path) -> Value {
    json!({
        "project": p.name,
        "file": path.display().to_string(),
        "screens": p.screens.iter().map(|s| json!({
            "id": s.id, "name": s.name, "device": s.device, "width": s.w, "height": s.h,
            "controls": s.controls.len(), "dark": s.dark, "is_component": s.is_component,
            "background": s.background, "notes": s.notes,
        })).collect::<Vec<_>>()
    })
}

fn resolve_screen_ref(p: &Project, v: &str) -> Option<String> {
    p.screen(v).map(|s| s.id.clone())
}

/// Applies the fields present in `v` to a control.
fn apply(c: &mut Control, v: &Value, p: &Project) -> Result<(), String> {
    let num = |k: &str| v.get(k).and_then(|x| x.as_f64()).map(|x| x as f32);
    let st = |k: &str| v.get(k).and_then(|x| x.as_str());
    let b = |k: &str| v.get(k).and_then(|x| x.as_bool());
    if let Some(t) = st("type") {
        let k = catalog::kind(t).ok_or_else(|| format!("unknown control type '{t}'; call list_control_types"))?;
        c.kind = k.id.to_string();
    }
    if let Some(x) = num("x") {
        c.x = x;
    }
    if let Some(x) = num("y") {
        c.y = x;
    }
    if let Some(x) = num("w").or_else(|| num("width")) {
        c.w = x.max(1.0);
    }
    if let Some(x) = num("h").or_else(|| num("height")) {
        c.h = x.max(1.0);
    }
    if let Some(t) = st("text") {
        c.text = t.to_string();
    }
    if let Some(s) = st("state") {
        c.state = State::parse(s).ok_or_else(|| format!("unknown state '{s}'"))?;
    }
    if let Some(x) = num("value") {
        c.value = x;
    }
    if let Some(s) = st("fill") {
        c.fill = if s.is_empty() { None } else { Some(s.to_string()) };
    }
    if let Some(s) = st("color") {
        c.color = if s.is_empty() { None } else { Some(s.to_string()) };
    }
    if let Some(x) = num("font_size") {
        c.font_size = if x > 0.0 { Some(x) } else { None };
    }
    if let Some(x) = b("bold") {
        c.bold = x;
    }
    if let Some(x) = b("italic") {
        c.italic = x;
    }
    if let Some(x) = b("underline") {
        c.underline = x;
    }
    if let Some(s) = st("fit") {
        c.fit = crate::images::Fit::parse(s).ok_or_else(|| format!("unknown fit '{s}' (stretch, contain or cover)"))?;
    }
    if let Some(s) = st("align") {
        c.align = match s {
            "left" => Align::Left,
            "center" | "centre" => Align::Center,
            "right" => Align::Right,
            _ => Align::Auto,
        };
    }
    if let Some(x) = num("opacity") {
        c.opacity = if x >= 1.0 { None } else { Some(x.clamp(0.05, 1.0)) };
    }
    if let Some(s) = st("link") {
        c.link = if s.is_empty() { None } else { Some(resolve_screen_ref(p, s).ok_or_else(|| format!("link: no screen '{s}'"))?) };
    }
    if let Some(s) = st("component") {
        c.component = if s.is_empty() { None } else { Some(resolve_screen_ref(p, s).ok_or_else(|| format!("component: no screen '{s}'"))?) };
    }
    if let Some(s) = st("name") {
        c.name = if s.is_empty() { None } else { Some(s.to_string()) };
    }
    if let Some(x) = b("locked") {
        c.locked = x;
    }
    if let Some(x) = b("hidden") {
        c.hidden = x;
    }
    Ok(())
}

fn call(name: &str, args: &Value, path: &Path) -> Result<Vec<Value>, String> {
    let mut p = load(path);
    let st = |k: &str| args.get(k).and_then(|x| x.as_str());
    let num = |k: &str| args.get(k).and_then(|x| x.as_f64()).map(|x| x as f32);
    let b = |k: &str| args.get(k).and_then(|x| x.as_bool());
    let save = |p: &Project| p.save(path);
    match name {
        "get_project" => Ok(text(serde_json::to_string_pretty(&summary(&p, path)).unwrap())),
        "list_control_types" => {
            let cat = st("category");
            let kinds: Vec<Value> = catalog::KINDS
                .iter()
                .filter(|k| cat.map(|c| k.cat.eq_ignore_ascii_case(c)).unwrap_or(true))
                .map(|k| json!({"type": k.id, "name": k.name, "category": k.cat, "w": k.w, "h": k.h, "default_text": k.text, "how_text_is_read": k.help}))
                .collect();
            let devices: Vec<Value> = catalog::DEVICES.iter().map(|d| json!({"device": d.id, "name": d.name, "width": d.w, "height": d.h})).collect();
            Ok(text(serde_json::to_string_pretty(&json!({
                "general_text_rules": "Lists: one item per line or comma separated as noted. Prefix * = active/selected item, - = disabled, :iconname = icon (e.g. ':home Home'). A tab character separates a trailing shortcut or detail.",
                "states": "normal, selected (checked/on/pressed/primary), disabled, focused (open/typing)",
                "control_types": kinds, "devices": devices, "icons": catalog::ICONS,
            })).unwrap()))
        }
        "get_screen" => {
            let i = screen_arg(&p, args)?;
            Ok(text(serde_json::to_string_pretty(&p.screens[i]).unwrap()))
        }
        "create_screen" => {
            let nm = st("name").unwrap_or("Screen");
            let dev = st("device").unwrap_or("phone_island");
            if catalog::device(dev).is_none() {
                return Err(format!("unknown device '{dev}'; see list_control_types"));
            }
            let id = p.add_screen(nm, dev);
            let bg = st("background").and_then(|b| resolve_screen_ref(&p, b));
            let s = p.screens.last_mut().unwrap();
            if let Some(w) = num("width") {
                s.w = w.max(20.0);
            }
            if let Some(h) = num("height") {
                s.h = h.max(20.0);
            }
            s.dark = b("dark").unwrap_or(false);
            s.is_component = b("is_component").unwrap_or(false);
            s.notes = st("notes").unwrap_or("").to_string();
            s.background = bg;
            let msg = format!("Created screen {id} '{nm}' ({}×{}).", s.w, s.h);
            save(&p)?;
            Ok(text(msg))
        }
        "update_screen" => {
            let i = screen_arg(&p, args)?;
            let bg = st("background").map(|b| if b.is_empty() { None } else { resolve_screen_ref(&p, b) });
            let s = &mut p.screens[i];
            if let Some(n) = st("name") {
                s.name = n.to_string();
            }
            if let Some(d) = st("device") {
                let dev = catalog::device(d).ok_or_else(|| format!("unknown device '{d}'"))?;
                s.device = dev.id.to_string();
                s.w = dev.w;
                s.h = dev.h;
            }
            if let Some(w) = num("width") {
                s.w = w.max(20.0);
            }
            if let Some(h) = num("height") {
                s.h = h.max(20.0);
            }
            if let Some(x) = b("dark") {
                s.dark = x;
            }
            if let Some(x) = b("is_component") {
                s.is_component = x;
            }
            if let Some(n) = st("notes") {
                s.notes = n.to_string();
            }
            if let Some(bg) = bg {
                s.background = bg;
            }
            let msg = format!("Updated {}.", s.id);
            save(&p)?;
            Ok(text(msg))
        }
        "duplicate_screen" => {
            let i = screen_arg(&p, args)?;
            let mut s = p.screens[i].clone();
            let mut n = p.screens.len() + 1;
            while p.screens.iter().any(|x| x.id == format!("s{n}")) {
                n += 1;
            }
            s.id = format!("s{n}");
            s.name = st("name").map(|x| x.to_string()).unwrap_or_else(|| format!("{} copy", s.name));
            for c in s.controls.iter_mut() {
                c.id = p.new_id();
            }
            let msg = format!("Created {} '{}'.", s.id, s.name);
            p.screens.insert(i + 1, s);
            save(&p)?;
            Ok(text(msg))
        }
        "delete_screen" => {
            let i = screen_arg(&p, args)?;
            if p.screens.len() == 1 {
                return Err("can't delete the only screen".into());
            }
            let s = p.screens.remove(i);
            save(&p)?;
            Ok(text(format!("Deleted {} '{}'.", s.id, s.name)))
        }
        "add_controls" => {
            let i = screen_arg(&p, args)?;
            let list = args.get("controls").and_then(|v| v.as_array()).ok_or("controls must be a list")?;
            let mut ids = Vec::new();
            for v in list {
                let t = v.get("type").and_then(|x| x.as_str()).ok_or("each control needs a type")?;
                if catalog::kind(t).is_none() {
                    return Err(format!("unknown control type '{t}'; call list_control_types"));
                }
                let mut c = p.make_control(t, 0.0, 0.0);
                apply(&mut c, v, &p)?;
                ids.push(c.id);
                p.screens[i].controls.push(c);
            }
            save(&p)?;
            Ok(text(format!("Added {} control(s) to {}: ids {:?}", ids.len(), p.screens[i].id, ids)))
        }
        "add_picture" => {
            use base64::Engine;
            let i = screen_arg(&p, args)?;
            let img = if let Some(path) = args.get("path").and_then(|v| v.as_str()) {
                let path = std::path::Path::new(path);
                if !crate::images::is_image_path(path) {
                    return Err("that file is not a picture (use PNG, JPEG, GIF, WebP, BMP, ICO, TIFF or SVG)".into());
                }
                let bytes = std::fs::read(path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
                let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                crate::images::Img::from_bytes(&name, bytes)?
            } else if let Some(data) = args.get("data").and_then(|v| v.as_str()) {
                let bytes = base64::engine::general_purpose::STANDARD.decode(data.trim()).map_err(|e| format!("data is not valid base64: {e}"))?;
                let name = args.get("name").and_then(|v| v.as_str()).unwrap_or("picture");
                crate::images::Img::from_bytes(name, bytes)?
            } else {
                return Err("give either path or data".into());
            };
            let num = |k: &str| args.get(k).and_then(|x| x.as_f64()).map(|x| x as f32);
            let size = match (num("width"), num("height")) {
                (Some(w), Some(h)) => Some((w, h)),
                (Some(w), None) => Some((w, w * img.h / img.w)),
                (None, Some(h)) => Some((h * img.w / img.h, h)),
                _ => None,
            };
            let at = match (num("x"), num("y")) {
                (Some(x), Some(y)) => {
                    let (w, h) = size.unwrap_or((img.w, img.h));
                    Some((x + w / 2.0, y + h / 2.0))
                }
                _ => None,
            };
            let id = p.add_picture(i, img, at, size);
            if let Some(f) = args.get("fit").and_then(|v| v.as_str()) {
                let fit = crate::images::Fit::parse(f).ok_or_else(|| format!("unknown fit '{f}'"))?;
                if let Some(c) = p.screens[i].controls.iter_mut().find(|c| c.id == id) {
                    c.fit = fit;
                }
            }
            save(&p)?;
            Ok(text(format!("Added picture to {}: control id {id}", p.screens[i].id)))
        }
        "update_controls" => {
            let i = screen_arg(&p, args)?;
            let list = args.get("updates").and_then(|v| v.as_array()).ok_or("updates must be a list")?;
            let snapshot = p.clone();
            let mut n = 0;
            for v in list {
                let id = v.get("id").and_then(|x| x.as_u64()).ok_or("each update needs an id")?;
                let c = p.screens[i].controls.iter_mut().find(|c| c.id == id).ok_or_else(|| format!("no control {id} on this screen"))?;
                apply(c, v, &snapshot)?;
                n += 1;
            }
            save(&p)?;
            Ok(text(format!("Updated {n} control(s).")))
        }
        "delete_controls" => {
            let i = screen_arg(&p, args)?;
            let ids: Vec<u64> = args.get("ids").and_then(|v| v.as_array()).map(|a| a.iter().filter_map(|x| x.as_u64()).collect()).unwrap_or_default();
            let before = p.screens[i].controls.len();
            p.screens[i].controls.retain(|c| !ids.contains(&c.id));
            let n = before - p.screens[i].controls.len();
            save(&p)?;
            Ok(text(format!("Deleted {n} control(s).")))
        }
        "clear_screen" => {
            let i = screen_arg(&p, args)?;
            p.screens[i].controls.clear();
            save(&p)?;
            Ok(text("Cleared."))
        }
        "render_screen" => {
            let i = screen_arg(&p, args)?;
            let frame = b("frame").unwrap_or(true);
            let sketch = b("sketch").unwrap_or(true);
            let scale = num("scale").unwrap_or(1.0).clamp(0.1, 4.0);
            let png = render::screen_png(&p, i, sketch, frame, scale)?;
            let s = &p.screens[i];
            Ok(vec![
                json!({"type": "image", "data": base64(&png), "mimeType": "image/png"}),
                json!({"type": "text", "text": format!("{} '{}' {}×{}, {} controls", s.id, s.name, s.w, s.h, s.controls.len())}),
            ])
        }
        "export" => {
            let fmt = st("format").ok_or("format is required")?;
            let out = PathBuf::from(st("path").ok_or("path is required")?);
            let sketch = b("sketch").unwrap_or(true);
            match fmt {
                "json" => p.save(&out)?,
                "html" => std::fs::write(&out, render::prototype_html(&p, sketch)).map_err(|e| e.to_string())?,
                "svg" | "png" => {
                    let i = screen_arg(&p, args)?;
                    if fmt == "svg" {
                        std::fs::write(&out, render::screen_svg(&p, i, sketch, true)).map_err(|e| e.to_string())?;
                    } else {
                        std::fs::write(&out, render::screen_png(&p, i, sketch, true, 2.0)?).map_err(|e| e.to_string())?;
                    }
                }
                _ => return Err(format!("unknown format '{fmt}'")),
            }
            Ok(text(format!("Wrote {}", out.display())))
        }
        "save_version" => {
            let nm = st("name").unwrap_or("Version");
            p.versions.push(model::Snapshot { name: nm.to_string(), time: model::now_secs(), screens: p.screens.clone() });
            save(&p)?;
            Ok(text(format!("Saved version '{nm}'.")))
        }
        _ => Err(format!("unknown tool '{name}'")),
    }
}

pub fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut s = String::with_capacity(data.len().div_ceil(3) * 4);
    for ch in data.chunks(3) {
        let n = (ch[0] as u32) << 16 | (*ch.get(1).unwrap_or(&0) as u32) << 8 | *ch.get(2).unwrap_or(&0) as u32;
        s.push(T[(n >> 18) as usize & 63] as char);
        s.push(T[(n >> 12) as usize & 63] as char);
        s.push(if ch.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        s.push(if ch.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    s
}
