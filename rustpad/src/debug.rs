//! Headless test hooks. With `RUSTPAD_SCRIPT=<file>` the app replays a script of
//! input events (keys, text, pointer) and can dump state or take screenshots.
//! This is how the UI is verified without a human at the keyboard.
//!
//! Script lines: `wait N`, `key ctrl+shift+s`, `text hello`, `press X Y`,
//! `release X Y`, `click X Y`, `move X Y`, `shot out.png`, `dump out.txt`,
//! `state out.json`, `cmd <json Cmd>`, `exit`.

use std::collections::VecDeque;

use eframe::egui::{self, Event, Key, Modifiers, PointerButton, RawInput};

pub struct Script {
    steps: VecDeque<String>,
    wait: u32,
    pub pending: Vec<String>,
    pub shot_path: Option<String>,
    mods: Modifiers,
    until: Option<std::time::Instant>,
}

fn parse_mods_key(s: &str) -> Option<(Modifiers, Key)> {
    let mut mods = Modifiers::NONE;
    let mut key = None;
    for part in s.split('+') {
        match part.to_lowercase().as_str() {
            "ctrl" | "cmd" | "command" => {
                mods.ctrl = true;
                mods.command = true;
            }
            "shift" => mods.shift = true,
            "alt" => mods.alt = true,
            other => {
                let mut cap = other.to_string();
                if let Some(f) = cap.get_mut(0..1) {
                    f.make_ascii_uppercase();
                }
                key = Key::from_name(other).or_else(|| Key::from_name(part)).or_else(|| Key::from_name(&cap));
            }
        }
    }
    key.map(|k| (mods, k))
}

impl Script {
    pub fn from_env() -> Option<Script> {
        let mut steps: VecDeque<String> = VecDeque::new();
        if let Ok(path) = std::env::var("RUSTPAD_SCRIPT") {
            let text = std::fs::read_to_string(path).ok()?;
            for l in text.lines() {
                let l = l.trim_end();
                if l.is_empty() || l.starts_with('#') {
                    continue;
                }
                steps.push_back(l.to_string());
            }
        } else if let Ok(p) = std::env::var("RUSTPAD_SCREENSHOT") {
            steps.push_back("wait 20".into());
            steps.push_back(format!("shot {p}"));
            steps.push_back("exit".into());
        } else {
            return None;
        }
        Some(Script { steps, wait: 8, pending: Vec::new(), shot_path: None, mods: Modifiers::NONE, until: None })
    }

    /// Called from `raw_input_hook`: feeds at most one step per frame.
    pub fn inject(&mut self, raw: &mut RawInput) {
        // no window manager under Xvfb: pretend the window has focus
        raw.focused = true;
        if self.wait > 0 {
            self.wait -= 1;
            return;
        }
        if let Some(t) = self.until {
            if std::time::Instant::now() < t {
                return;
            }
            self.until = None;
        }
        let Some(step) = self.steps.pop_front() else { return };
        let (op, rest) = step.split_once(' ').unwrap_or((step.as_str(), ""));
        match op {
            "wait" => self.wait = rest.trim().parse().unwrap_or(1),
            "sleep" => self.until = Some(std::time::Instant::now() + std::time::Duration::from_millis(rest.trim().parse().unwrap_or(100))),
            "key" => {
                if let Some((m, k)) = parse_mods_key(rest.trim()) {
                    raw.events.push(Event::ModifiersChanged(m));
                    raw.events.push(Event::Key { key: k, physical_key: Some(k), pressed: true, repeat: false, modifiers: m });
                    raw.events.push(Event::Key { key: k, physical_key: Some(k), pressed: false, repeat: false, modifiers: m });
                    raw.events.push(Event::ModifiersChanged(Modifiers::NONE));
                }
            }
            "paste" => raw.events.push(Event::Paste(rest.replace("\\n", "\n"))),
            "text" => {
                raw.events.push(Event::Text(rest.replace("\\t", "\t")));
            }
            "move" | "press" | "release" | "click" | "rclick" => {
                let n: Vec<f32> = rest.split_whitespace().filter_map(|v| v.parse().ok()).collect();
                if n.len() >= 2 {
                    let pos = egui::pos2(n[0], n[1]);
                    // optional trailing modifiers: ctrl / shift / alt
                    let mut m = Modifiers::NONE;
                    for w in rest.split_whitespace().skip(2) {
                        match w {
                            "ctrl" => {
                                m.ctrl = true;
                                m.command = true;
                            }
                            "shift" => m.shift = true,
                            "alt" => m.alt = true,
                            _ => {}
                        }
                    }
                    self.mods = m;
                    raw.events.push(Event::ModifiersChanged(m));
                    raw.events.push(Event::PointerMoved(pos));
                    let btn = if op == "rclick" { PointerButton::Secondary } else { PointerButton::Primary };
                    match op {
                        "press" => raw.events.push(Event::PointerButton { pos, button: btn, pressed: true, modifiers: m }),
                        "release" => {
                            raw.events.push(Event::PointerButton { pos, button: btn, pressed: false, modifiers: m });
                            raw.events.push(Event::ModifiersChanged(Modifiers::NONE));
                        }
                        "click" | "rclick" => {
                            raw.events.push(Event::PointerButton { pos, button: btn, pressed: true, modifiers: m });
                            self.steps.push_front(format!("release {} {}", n[0], n[1]));
                            if op == "rclick" {
                                self.steps.pop_front();
                                self.steps.push_front(format!("rrelease {} {}", n[0], n[1]));
                            }
                        }
                        _ => {}
                    }
                }
            }
            "rrelease" => {
                let n: Vec<f32> = rest.split_whitespace().filter_map(|v| v.parse().ok()).collect();
                if n.len() >= 2 {
                    let pos = egui::pos2(n[0], n[1]);
                    raw.events.push(Event::PointerButton { pos, button: PointerButton::Secondary, pressed: false, modifiers: Modifiers::NONE });
                }
            }
            _ => self.pending.push(step),
        }
    }
}
