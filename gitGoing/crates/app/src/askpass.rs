//! Credential prompt. Git and ssh start our executable (see
//! `gitgoing_core::gitcmd`) with the prompt text as its first argument and read
//! the answer from stdout; this shows a small window to collect it.

use eframe::egui;
use std::sync::{Arc, Mutex};

struct Prompt {
    text: String,
    secret: bool,
    value: String,
    answer: Arc<Mutex<Option<String>>>,
    focused: bool,
}

impl eframe::App for Prompt {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut done = false;
        let mut cancel = false;
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label(&self.text);
            ui.add_space(6.0);
            let r = ui.add(
                egui::TextEdit::singleline(&mut self.value)
                    .password(self.secret)
                    .desired_width(f32::INFINITY),
            );
            if !self.focused {
                r.request_focus();
                self.focused = true;
            }
            if r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter)) {
                done = true;
            }
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("OK").clicked() {
                    done = true;
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
        if ctx.input(|i| i.key_pressed(egui::Key::Escape)) {
            cancel = true;
        }
        if done {
            *self.answer.lock().unwrap() = Some(self.value.clone());
        }
        if done || cancel {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }
}

/// Whether `prompt` is asking for something that should not be shown on screen.
fn is_secret(prompt: &str) -> bool {
    let p = prompt.to_lowercase();
    ["password", "passphrase", "pin", "token"].iter().any(|w| p.contains(w))
}

/// Run as an askpass helper: print the answer and exit 0, or exit 1 if cancelled.
pub fn run() -> ! {
    let text = std::env::args().nth(1).unwrap_or_else(|| "Enter credentials".to_owned());
    let answer = Arc::new(Mutex::new(None));
    let app = Prompt {
        secret: is_secret(&text),
        text: text.clone(),
        value: String::new(),
        answer: answer.clone(),
        focused: false,
    };
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("gitgoing: credentials required")
            .with_inner_size([460.0, 150.0])
            .with_always_on_top(),
        persist_window: false,
        ..Default::default()
    };
    let _ = eframe::run_native("gitgoing-askpass", options, Box::new(move |_| Ok(Box::new(app))));
    let result = answer.lock().unwrap().take();
    match result {
        Some(v) => {
            println!("{v}");
            std::process::exit(0)
        }
        None => std::process::exit(1),
    }
}

#[cfg(test)]
mod tests {
    use super::is_secret;

    #[test]
    fn hides_passwords_but_not_usernames() {
        assert!(is_secret("Password for 'https://user@host': "));
        assert!(is_secret("Enter passphrase for key '/home/u/.ssh/id_ed25519': "));
        assert!(!is_secret("Username for 'https://github.com': "));
        assert!(!is_secret("Are you sure you want to continue connecting (yes/no)? "));
    }
}
