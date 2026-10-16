//! Font setup: the terminal font family (user font or Hack) plus CJK / symbol fallbacks.

use std::sync::Arc;

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};

use crate::config::Settings;
use crate::view::FONT_FAMILY;

fn first_existing(paths: &[&str]) -> Option<std::path::PathBuf> {
    paths.iter().map(std::path::PathBuf::from).find(|p| p.exists())
}

pub fn install(ctx: &egui::Context, s: &Settings) {
    let mut fonts = FontDefinitions::default();
    let mut family: Vec<String> = fonts.families.get(&FontFamily::Monospace).cloned().unwrap_or_default();

    if !s.font_path.trim().is_empty()
        && let Ok(bytes) = std::fs::read(s.font_path.trim())
    {
        fonts.font_data.insert("user-mono".into(), Arc::new(FontData::from_owned(bytes)));
        family.insert(0, "user-mono".into());
    }

    let fallbacks = [
        (
            "cjk-fallback",
            first_existing(&[
                "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
                "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
                "/System/Library/Fonts/PingFang.ttc",
                "/System/Library/Fonts/Hiragino Sans GB.ttc",
                "C:\\Windows\\Fonts\\msyh.ttc",
                "C:\\Windows\\Fonts\\YuGothR.ttc",
            ]),
        ),
        (
            "symbol-fallback",
            first_existing(&[
                "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
                "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
                "/System/Library/Fonts/Menlo.ttc",
                "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
                "C:\\Windows\\Fonts\\seguisym.ttf",
            ]),
        ),
    ];
    for (name, path) in fallbacks {
        if let Some(bytes) = path.and_then(|p| std::fs::read(p).ok()) {
            fonts.font_data.insert(name.into(), Arc::new(FontData::from_owned(bytes)));
            family.push(name.into());
            fonts.families.entry(FontFamily::Proportional).or_default().push(name.into());
        }
    }
    fonts.families.insert(FontFamily::Name(FONT_FAMILY.into()), family);
    ctx.set_fonts(fonts);
}
