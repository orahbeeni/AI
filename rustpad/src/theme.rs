//! Color themes (editor chrome + syntax colors). Built-in themes plus JSON
//! themes dropped into `<config>/themes/`.

use eframe::egui::Color32;
use serde::{Deserialize, Serialize};

use crate::lang::Style;

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn c(self) -> Color32 {
        Color32::from_rgb(self.0, self.1, self.2)
    }
    pub fn a(self, alpha: u8) -> Color32 {
        Color32::from_rgba_unmultiplied(self.0, self.1, self.2, alpha)
    }
}

const fn rgb(hex: u32) -> Rgb {
    Rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub const STYLE_COUNT: usize = Style::Key as usize + 1;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Theme {
    pub name: String,
    pub dark: bool,
    pub bg: Rgb,
    pub fg: Rgb,
    pub caret: Rgb,
    pub current_line: Rgb,
    pub selection: Rgb,
    pub gutter_bg: Rgb,
    pub gutter_fg: Rgb,
    pub gutter_active_fg: Rgb,
    pub fold_marker: Rgb,
    pub edge: Rgb,
    pub guide: Rgb,
    pub whitespace: Rgb,
    pub bracket_match: Rgb,
    pub bracket_bad: Rgb,
    pub smart_highlight: Rgb,
    pub find_highlight: Rgb,
    pub marks: [Rgb; 5],
    pub bookmark: Rgb,
    /// Colors indexed by `Style as usize`.
    pub syntax: Vec<Rgb>,
}

impl Theme {
    pub fn style_color(&self, s: Style) -> Color32 {
        match s {
            Style::Default => self.fg.c(),
            _ => self.syntax.get(s as usize).copied().unwrap_or(self.fg).c(),
        }
    }
}

struct Syn {
    comment: u32,
    doc: u32,
    string: u32,
    ch: u32,
    number: u32,
    keyword: u32,
    ty: u32,
    literal: u32,
    builtin: u32,
    function: u32,
    operator: u32,
    preproc: u32,
    variable: u32,
    tag: u32,
    attr: u32,
    entity: u32,
    heading: u32,
    link: u32,
    code: u32,
    quote: u32,
    added: u32,
    removed: u32,
    hunk: u32,
    section: u32,
    key: u32,
}

fn syntax_table(fg: u32, s: &Syn) -> Vec<Rgb> {
    let mut v = vec![rgb(fg); STYLE_COUNT];
    let mut set = |st: Style, c: u32| v[st as usize] = rgb(c);
    set(Style::Comment, s.comment);
    set(Style::DocComment, s.doc);
    set(Style::String, s.string);
    set(Style::Char, s.ch);
    set(Style::Number, s.number);
    set(Style::Keyword, s.keyword);
    set(Style::Type, s.ty);
    set(Style::Literal, s.literal);
    set(Style::Builtin, s.builtin);
    set(Style::Function, s.function);
    set(Style::Operator, s.operator);
    set(Style::Preproc, s.preproc);
    set(Style::Variable, s.variable);
    set(Style::Tag, s.tag);
    set(Style::Attr, s.attr);
    set(Style::Entity, s.entity);
    set(Style::Heading, s.heading);
    set(Style::Bold, s.keyword);
    set(Style::Italic, s.variable);
    set(Style::Link, s.link);
    set(Style::Code, s.code);
    set(Style::Quote, s.quote);
    set(Style::Added, s.added);
    set(Style::Removed, s.removed);
    set(Style::Hunk, s.hunk);
    set(Style::Section, s.section);
    set(Style::Key, s.key);
    v
}

pub fn builtin() -> Vec<Theme> {
    let light = Theme {
        name: "Default".into(),
        dark: false,
        bg: rgb(0xffffff),
        fg: rgb(0x000000),
        caret: rgb(0x000000),
        current_line: rgb(0xe8e8ff),
        selection: rgb(0xc0d8ff),
        gutter_bg: rgb(0xe4e4e4),
        gutter_fg: rgb(0x808080),
        gutter_active_fg: rgb(0x000000),
        fold_marker: rgb(0x808080),
        edge: rgb(0xe0b0b0),
        guide: rgb(0xd0d0d0),
        whitespace: rgb(0xb0b0b0),
        bracket_match: rgb(0xa0e0a0),
        bracket_bad: rgb(0xf0a0a0),
        smart_highlight: rgb(0xbfffbf),
        find_highlight: rgb(0xffd060),
        marks: [rgb(0xffff00), rgb(0xff8040), rgb(0x80ff80), rgb(0x80c0ff), rgb(0xff80ff)],
        bookmark: rgb(0x2060d0),
        syntax: syntax_table(
            0x000000,
            &Syn {
                comment: 0x008000,
                doc: 0x008080,
                string: 0x808080,
                ch: 0x808080,
                number: 0xff8000,
                keyword: 0x0000ff,
                ty: 0x8000ff,
                literal: 0xff0080,
                builtin: 0x804000,
                function: 0x000080,
                operator: 0x000080,
                preproc: 0x804000,
                variable: 0x8000ff,
                tag: 0x0000ff,
                attr: 0xff0000,
                entity: 0xff8000,
                heading: 0x0000a0,
                link: 0x0000ee,
                code: 0x804000,
                quote: 0x008000,
                added: 0x008000,
                removed: 0xc00000,
                hunk: 0x8000ff,
                section: 0x0000ff,
                key: 0x000080,
            },
        ),
    };

    let obsidian = Theme {
        name: "Dark".into(),
        dark: true,
        bg: rgb(0x1e1f22),
        fg: rgb(0xd4d4d4),
        caret: rgb(0xffffff),
        current_line: rgb(0x2a2d33),
        selection: rgb(0x214283),
        gutter_bg: rgb(0x25272b),
        gutter_fg: rgb(0x6b7078),
        gutter_active_fg: rgb(0xc0c4cc),
        fold_marker: rgb(0x80848c),
        edge: rgb(0x553333),
        guide: rgb(0x3a3d42),
        whitespace: rgb(0x555a62),
        bracket_match: rgb(0x2f5f3f),
        bracket_bad: rgb(0x7a2f2f),
        smart_highlight: rgb(0x38502f),
        find_highlight: rgb(0x7a5c10),
        marks: [rgb(0x808000), rgb(0xa04820), rgb(0x2f7f2f), rgb(0x2060a0), rgb(0x8f3f8f)],
        bookmark: rgb(0x5a9bff),
        syntax: syntax_table(
            0xd4d4d4,
            &Syn {
                comment: 0x6a9955,
                doc: 0x5f8f6a,
                string: 0xce9178,
                ch: 0xd7ba7d,
                number: 0xb5cea8,
                keyword: 0x569cd6,
                ty: 0x4ec9b0,
                literal: 0x9cdcfe,
                builtin: 0xdcdcaa,
                function: 0xdcdcaa,
                operator: 0xc8c8c8,
                preproc: 0xc586c0,
                variable: 0x9cdcfe,
                tag: 0x569cd6,
                attr: 0x9cdcfe,
                entity: 0xd7ba7d,
                heading: 0x569cd6,
                link: 0x6aa6f8,
                code: 0xce9178,
                quote: 0x6a9955,
                added: 0x7ec07e,
                removed: 0xe07070,
                hunk: 0xc586c0,
                section: 0xc586c0,
                key: 0x9cdcfe,
            },
        ),
    };

    let mut monokai = obsidian.clone();
    monokai.name = "Monokai".into();
    monokai.bg = rgb(0x272822);
    monokai.fg = rgb(0xf8f8f2);
    monokai.caret = rgb(0xf8f8f0);
    monokai.current_line = rgb(0x3e3d32);
    monokai.selection = rgb(0x49483e);
    monokai.gutter_bg = rgb(0x2d2e27);
    monokai.guide = rgb(0x49483e);
    monokai.syntax = syntax_table(
        0xf8f8f2,
        &Syn {
            comment: 0x75715e,
            doc: 0x8a8669,
            string: 0xe6db74,
            ch: 0xe6db74,
            number: 0xae81ff,
            keyword: 0xf92672,
            ty: 0x66d9ef,
            literal: 0xae81ff,
            builtin: 0x66d9ef,
            function: 0xa6e22e,
            operator: 0xf92672,
            preproc: 0xf92672,
            variable: 0xfd971f,
            tag: 0xf92672,
            attr: 0xa6e22e,
            entity: 0xae81ff,
            heading: 0xa6e22e,
            link: 0x66d9ef,
            code: 0xe6db74,
            quote: 0x75715e,
            added: 0xa6e22e,
            removed: 0xf92672,
            hunk: 0xae81ff,
            section: 0xf92672,
            key: 0xa6e22e,
        },
    );

    let mut sol_dark = obsidian.clone();
    sol_dark.name = "Solarized Dark".into();
    sol_dark.bg = rgb(0x002b36);
    sol_dark.fg = rgb(0x93a1a1);
    sol_dark.caret = rgb(0x93a1a1);
    sol_dark.current_line = rgb(0x073642);
    sol_dark.selection = rgb(0x0a4a5a);
    sol_dark.gutter_bg = rgb(0x073642);
    sol_dark.gutter_fg = rgb(0x586e75);
    sol_dark.guide = rgb(0x0f4652);
    sol_dark.syntax = syntax_table(
        0x93a1a1,
        &Syn {
            comment: 0x586e75,
            doc: 0x657b83,
            string: 0x2aa198,
            ch: 0x2aa198,
            number: 0xd33682,
            keyword: 0x859900,
            ty: 0xb58900,
            literal: 0xcb4b16,
            builtin: 0x268bd2,
            function: 0x268bd2,
            operator: 0x93a1a1,
            preproc: 0xcb4b16,
            variable: 0x6c71c4,
            tag: 0x268bd2,
            attr: 0x93a1a1,
            entity: 0xcb4b16,
            heading: 0xb58900,
            link: 0x268bd2,
            code: 0x2aa198,
            quote: 0x586e75,
            added: 0x859900,
            removed: 0xdc322f,
            hunk: 0x6c71c4,
            section: 0xb58900,
            key: 0x268bd2,
        },
    );

    let mut sol_light = light.clone();
    sol_light.name = "Solarized Light".into();
    sol_light.bg = rgb(0xfdf6e3);
    sol_light.fg = rgb(0x586e75);
    sol_light.caret = rgb(0x586e75);
    sol_light.current_line = rgb(0xeee8d5);
    sol_light.selection = rgb(0xd8e4d0);
    sol_light.gutter_bg = rgb(0xeee8d5);
    sol_light.gutter_fg = rgb(0x93a1a1);
    sol_light.guide = rgb(0xe3dcc6);
    sol_light.syntax = sol_dark.syntax.clone();
    sol_light.syntax[Style::Comment as usize] = rgb(0x93a1a1);
    sol_light.syntax[Style::DocComment as usize] = rgb(0x93a1a1);
    sol_light.syntax[Style::Operator as usize] = rgb(0x586e75);
    sol_light.syntax[Style::Attr as usize] = rgb(0x586e75);

    vec![light, obsidian, monokai, sol_dark, sol_light]
}

pub fn load_all(config_dir: &std::path::Path) -> Vec<Theme> {
    let mut all = builtin();
    if let Ok(rd) = std::fs::read_dir(config_dir.join("themes")) {
        for e in rd.flatten() {
            if let Ok(text) = std::fs::read_to_string(e.path()) {
                if let Ok(mut t) = serde_json::from_str::<Theme>(&text) {
                    t.syntax.resize(STYLE_COUNT, t.fg);
                    all.retain(|x| x.name != t.name);
                    all.push(t);
                }
            }
        }
    }
    all
}
