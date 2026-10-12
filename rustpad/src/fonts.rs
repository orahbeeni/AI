//! Font setup: optional user monospace font plus CJK / symbol fallbacks found on the system.

use eframe::egui::{self, FontData, FontDefinitions, FontFamily};

use crate::settings::Settings;

fn first_existing(paths: &[&str]) -> Option<std::path::PathBuf> {
    paths.iter().map(std::path::PathBuf::from).find(|p| p.exists())
}

pub fn install(ctx: &egui::Context, set: &Settings) {
    let mut fonts = FontDefinitions::default();

    if !set.font_name.is_empty() {
        let p = std::path::PathBuf::from(&set.font_name);
        if let Ok(bytes) = std::fs::read(&p) {
            fonts.font_data.insert("user-mono".into(), std::sync::Arc::new(FontData::from_owned(bytes)));
            fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "user-mono".into());
        }
    }

    let cjk = first_existing(&[
        "/usr/share/fonts/opentype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/noto/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/google-noto-cjk/NotoSansCJK-Regular.ttc",
        "/usr/share/fonts/truetype/wqy/wqy-microhei.ttc",
        "/System/Library/Fonts/PingFang.ttc",
        "/System/Library/Fonts/Hiragino Sans GB.ttc",
        "/System/Library/Fonts/AppleSDGothicNeo.ttc",
        "C:\\Windows\\Fonts\\msyh.ttc",
        "C:\\Windows\\Fonts\\YuGothR.ttc",
        "C:\\Windows\\Fonts\\malgun.ttf",
        "C:\\Windows\\Fonts\\simsun.ttc",
    ]);
    if let Some(p) = cjk {
        if let Ok(bytes) = std::fs::read(&p) {
            fonts.font_data.insert("cjk-fallback".into(), std::sync::Arc::new(FontData::from_owned(bytes)));
            for fam in [FontFamily::Monospace, FontFamily::Proportional] {
                fonts.families.entry(fam).or_default().push("cjk-fallback".into());
            }
        }
    }
    let symbols = first_existing(&[
        "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
        "/usr/share/fonts/TTF/DejaVuSans.ttf",
        "/System/Library/Fonts/Supplemental/Arial Unicode.ttf",
        "C:\\Windows\\Fonts\\seguisym.ttf",
    ]);
    if let Some(p) = symbols {
        if let Ok(bytes) = std::fs::read(&p) {
            fonts.font_data.insert("symbol-fallback".into(), std::sync::Arc::new(FontData::from_owned(bytes)));
            for fam in [FontFamily::Monospace, FontFamily::Proportional] {
                fonts.families.entry(fam).or_default().push("symbol-fallback".into());
            }
        }
    }
    ctx.set_fonts(fonts);
}
