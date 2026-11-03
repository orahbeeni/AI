//! The project file: screens made of controls. Shared by the editor and the MCP server.

use crate::catalog;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum State {
    #[default]
    Normal,
    Selected,
    Disabled,
    Focused,
}

impl State {
    pub const ALL: [State; 4] = [State::Normal, State::Selected, State::Disabled, State::Focused];
    pub fn label(self) -> &'static str {
        match self {
            State::Normal => "Normal",
            State::Selected => "Selected / on",
            State::Disabled => "Disabled",
            State::Focused => "Focused",
        }
    }
    pub fn parse(s: &str) -> Option<State> {
        match s.to_ascii_lowercase().as_str() {
            "normal" => Some(State::Normal),
            "selected" | "on" | "active" | "checked" => Some(State::Selected),
            "disabled" | "off" => Some(State::Disabled),
            "focused" | "focus" => Some(State::Focused),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Align {
    #[default]
    Auto,
    Left,
    Center,
    Right,
}

fn is_default<T: Default + PartialEq>(v: &T) -> bool {
    *v == T::default()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Control {
    pub id: u64,
    #[serde(rename = "type")]
    pub kind: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    #[serde(default)]
    pub text: String,
    #[serde(default, skip_serializing_if = "is_default")]
    pub state: State,
    #[serde(default, skip_serializing_if = "is_default")]
    pub value: f32,
    /// Background colour as "#rrggbb".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fill: Option<String>,
    /// Text and line colour as "#rrggbb".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub font_size: Option<f32>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub bold: bool,
    #[serde(default, skip_serializing_if = "is_default")]
    pub italic: bool,
    #[serde(default, skip_serializing_if = "is_default")]
    pub underline: bool,
    #[serde(default, skip_serializing_if = "is_default")]
    pub align: Align,
    /// 0..1, 1 is solid.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub opacity: Option<f32>,
    /// Screen id this control opens when clicked in present mode.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<String>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub locked: bool,
    #[serde(default, skip_serializing_if = "is_default")]
    pub hidden: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub group: Option<u64>,
    /// For "component" controls: the component screen it shows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub component: Option<String>,
    /// Layer name shown in the layers list.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
}

impl Control {
    pub fn contains(&self, x: f32, y: f32) -> bool {
        x >= self.x && y >= self.y && x <= self.x + self.w && y <= self.y + self.h
    }
    pub fn layer_name(&self) -> String {
        if let Some(n) = &self.name {
            if !n.is_empty() {
                return n.clone();
            }
        }
        let k = catalog::kind(&self.kind).map(|k| k.name).unwrap_or(&self.kind);
        let first = self.text.lines().next().unwrap_or("").split(',').next().unwrap_or("");
        let first = crate::draw::parse_item(first).label;
        if first.is_empty() || first.chars().count() > 24 {
            k.to_string()
        } else {
            format!("{k}: {first}")
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Screen {
    pub id: String,
    pub name: String,
    #[serde(default = "default_device")]
    pub device: String,
    pub w: f32,
    pub h: f32,
    #[serde(default)]
    pub controls: Vec<Control>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub notes: String,
    /// Another screen drawn underneath this one (a master / template).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub background: Option<String>,
    /// Component screens can be placed inside other screens and update everywhere.
    #[serde(default, skip_serializing_if = "is_default")]
    pub is_component: bool,
    /// Draw this screen as a dark-mode wireframe.
    #[serde(default, skip_serializing_if = "is_default")]
    pub dark: bool,
}

fn default_device() -> String {
    "none".into()
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Snapshot {
    pub name: String,
    pub time: u64,
    pub screens: Vec<Screen>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Project {
    #[serde(default = "one")]
    pub version: u32,
    pub name: String,
    pub screens: Vec<Screen>,
    #[serde(default = "one64")]
    pub next_id: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub versions: Vec<Snapshot>,
}

fn one() -> u32 {
    1
}
fn one64() -> u64 {
    1
}

pub fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

impl Project {
    pub fn empty() -> Project {
        let mut p = Project { version: 1, name: "Untitled".into(), screens: vec![], next_id: 1, versions: vec![] };
        p.add_screen("Home", "phone_island");
        p
    }

    pub fn new_id(&mut self) -> u64 {
        let max = self.screens.iter().flat_map(|s| s.controls.iter().map(|c| c.id)).max().unwrap_or(0);
        self.next_id = self.next_id.max(max + 1);
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    pub fn screen_idx(&self, key: &str) -> Option<usize> {
        self.screens
            .iter()
            .position(|s| s.id == key)
            .or_else(|| self.screens.iter().position(|s| s.name.eq_ignore_ascii_case(key)))
            .or_else(|| key.parse::<usize>().ok().filter(|i| *i < self.screens.len()))
    }

    pub fn screen(&self, key: &str) -> Option<&Screen> {
        self.screen_idx(key).map(|i| &self.screens[i])
    }

    pub fn add_screen(&mut self, name: &str, device: &str) -> String {
        let mut n = self.screens.len() + 1;
        while self.screens.iter().any(|s| s.id == format!("s{n}")) {
            n += 1;
        }
        let id = format!("s{n}");
        let dev = catalog::device(device).unwrap_or(&catalog::DEVICES[0]);
        self.screens.push(Screen {
            id: id.clone(),
            name: name.into(),
            device: dev.id.into(),
            w: dev.w,
            h: dev.h,
            controls: vec![],
            notes: String::new(),
            background: None,
            is_component: false,
            dark: false,
        });
        id
    }

    /// A new control with the catalog's default size, text and value.
    pub fn make_control(&mut self, kind: &str, x: f32, y: f32) -> Control {
        let k = catalog::kind(kind);
        Control {
            id: self.new_id(),
            kind: k.map(|k| k.id.to_string()).unwrap_or_else(|| kind.to_string()),
            x,
            y,
            w: k.map(|k| k.w).unwrap_or(120.0),
            h: k.map(|k| k.h).unwrap_or(40.0),
            text: k.map(|k| k.text.to_string()).unwrap_or_default(),
            state: State::Normal,
            value: k.map(|k| k.value).unwrap_or(0.0),
            fill: None,
            color: None,
            font_size: None,
            bold: false,
            italic: false,
            underline: false,
            align: Align::Auto,
            opacity: None,
            link: None,
            locked: false,
            hidden: false,
            group: None,
            component: None,
            name: None,
        }
    }

    pub fn load(path: &Path) -> Result<Project, String> {
        let s = std::fs::read_to_string(path).map_err(|e| format!("can't read {}: {e}", path.display()))?;
        let mut p: Project = serde_json::from_str(&s).map_err(|e| format!("not a project file: {e}"))?;
        if p.screens.is_empty() {
            p.add_screen("Home", "phone_island");
        }
        Ok(p)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let s = serde_json::to_string_pretty(self).map_err(|e| e.to_string())?;
        let tmp = path.with_extension("tmp");
        std::fs::write(&tmp, s).map_err(|e| format!("can't write {}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, path).map_err(|e| format!("can't save {}: {e}", path.display()))
    }
}

pub fn parse_hex(s: &str) -> Option<[u8; 4]> {
    let h = s.trim().trim_start_matches('#');
    let v = |i: usize| u8::from_str_radix(h.get(i..i + 2)?, 16).ok();
    match h.len() {
        6 => Some([v(0)?, v(2)?, v(4)?, 255]),
        8 => Some([v(0)?, v(2)?, v(4)?, v(6)?]),
        3 => {
            let c = |i: usize| u8::from_str_radix(&h[i..i + 1], 16).ok().map(|x| x * 17);
            Some([c(0)?, c(1)?, c(2)?, 255])
        }
        _ => None,
    }
}

pub fn to_hex(c: [u8; 4]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// Where the app keeps its own files.
pub fn data_dir() -> PathBuf {
    dirs::data_dir().unwrap_or_else(std::env::temp_dir).join("Sketchframe")
}

pub fn autosave_path() -> PathBuf {
    data_dir().join("autosave.sketch.json")
}

/// The file the editor has open, so the MCP server edits the same one.
pub fn current_pointer() -> PathBuf {
    data_dir().join("current.txt")
}

pub fn remember_current(path: &Path) {
    let _ = std::fs::create_dir_all(data_dir());
    let _ = std::fs::write(current_pointer(), path.display().to_string());
}

pub fn current_file() -> PathBuf {
    std::fs::read_to_string(current_pointer())
        .ok()
        .map(|s| PathBuf::from(s.trim()))
        .filter(|p| p.exists())
        .unwrap_or_else(autosave_path)
}

/// A small demo project so a fresh start isn't empty.
pub fn sample() -> Project {
    let mut p = Project { version: 1, name: "Sample app".into(), screens: vec![], next_id: 1, versions: vec![] };
    let login = p.add_screen("Sign in", "phone_island");
    let home = p.add_screen("Home feed", "phone_island");
    let web = p.add_screen("Web dashboard", "browser");
    let header = p.add_screen("Header", "none");
    {
        let i = p.screen_idx(&header).unwrap();
        p.screens[i].is_component = true;
        p.screens[i].w = 393.0;
        p.screens[i].h = 56.0;
    }
    let add = |p: &mut Project, s: &str, kind: &str, x: f32, y: f32, w: f32, h: f32, text: &str| -> u64 {
        let mut c = p.make_control(kind, x, y);
        c.w = w;
        c.h = h;
        if !text.is_empty() {
            c.text = text.into();
        }
        let id = c.id;
        let i = p.screen_idx(s).unwrap();
        p.screens[i].controls.push(c);
        id
    };
    let set = |p: &mut Project, s: &str, id: u64, f: &dyn Fn(&mut Control)| {
        let i = p.screen_idx(s).unwrap();
        if let Some(c) = p.screens[i].controls.iter_mut().find(|c| c.id == id) {
            f(c);
        }
    };

    add(&mut p, &header, "app_bar", 0.0, 0.0, 393.0, 56.0, ":menu Discover :search");

    // Sign in
    add(&mut p, &login, "status_bar", 0.0, 0.0, 393.0, 54.0, "");
    add(&mut p, &login, "image", 136.0, 110.0, 120.0, 120.0, "");
    add(&mut p, &login, "heading", 40.0, 260.0, 313.0, 44.0, "Welcome back");
    add(&mut p, &login, "label", 40.0, 306.0, 313.0, 24.0, "Sign in to continue");
    add(&mut p, &login, "text_input", 40.0, 360.0, 313.0, 48.0, "Email");
    add(&mut p, &login, "password", 40.0, 424.0, 313.0, 48.0, "Password");
    let b = add(&mut p, &login, "button_primary", 40.0, 500.0, 313.0, 52.0, "Sign in");
    set(&mut p, &login, b, &|c| c.link = Some(home.clone()));
    add(&mut p, &login, "link", 130.0, 570.0, 140.0, 24.0, "Forgot password?");
    add(&mut p, &login, "button", 40.0, 640.0, 313.0, 52.0, ":phone Continue with phone");
    add(&mut p, &login, "label", 90.0, 760.0, 240.0, 24.0, "New here? Create an account");

    // Home feed
    add(&mut p, &home, "status_bar", 0.0, 0.0, 393.0, 54.0, "");
    let comp = add(&mut p, &home, "component", 0.0, 54.0, 393.0, 56.0, "");
    set(&mut p, &home, comp, &|c| c.component = Some(header.clone()));
    add(&mut p, &home, "chips", 16.0, 122.0, 360.0, 36.0, "*All,Music,Podcasts,Books");
    add(&mut p, &home, "card", 16.0, 172.0, 361.0, 250.0, "Weekend playlist\nFresh picks for you, updated every Friday.");
    add(&mut p, &home, "list", 16.0, 436.0, 361.0, 200.0, ":heart Liked songs\n:clock Recently played\n:download Downloads\n:bookmark Saved for later");
    add(&mut p, &home, "fab", 313.0, 670.0, 56.0, 56.0, "plus");
    let nav = add(&mut p, &home, "bottom_nav", 0.0, 744.0, 393.0, 74.0, "*:home Home,:search Search,:heart Saved,:user Profile");
    set(&mut p, &home, nav, &|c| c.link = Some(login.clone()));

    // Web dashboard
    add(&mut p, &web, "navbar", 0.0, 0.0, 1280.0, 64.0, "Acme\n*Dashboard,Reports,Customers,Settings\nSign out");
    add(&mut p, &web, "sidebar", 0.0, 64.0, 240.0, 736.0, "*:grid Overview\n:list Orders\n:user Customers\n:file Invoices\n:settings Settings");
    add(&mut p, &web, "heading", 272.0, 92.0, 400.0, 40.0, "Overview");
    add(&mut p, &web, "segmented", 940.0, 96.0, 300.0, 36.0, "Day,*Week,Month");
    for (i, (t, v)) in [("Revenue", "$24.3k"), ("Orders", "1,284"), ("New users", "312")].iter().enumerate() {
        add(&mut p, &web, "stat", 272.0 + i as f32 * 328.0, 156.0, 304.0, 110.0, &format!("{t}\n{v}\n+12%"));
    }
    add(&mut p, &web, "chart_line", 272.0, 290.0, 632.0, 250.0, "30,45,40,60,55,75,70,90");
    add(&mut p, &web, "chart_pie", 928.0, 290.0, 312.0, 250.0, "45,30,25");
    add(&mut p, &web, "table", 272.0, 564.0, 968.0, 200.0, "Order,Customer,Status,Total\n#1042,Dana R.,Shipped,$120\n#1043,Lee M.,Pending,$64\n#1044,Sam K.,Refunded,$32\n#1045,Ari P.,Shipped,$210");
    add(&mut p, &web, "callout", 896.0, 92.0, 30.0, 30.0, "1");
    {
        let i = p.screen_idx(&web).unwrap();
        p.screens[i].notes = "1. Should the chart default to weekly? Ask the team.".into();
    }
    p
}
