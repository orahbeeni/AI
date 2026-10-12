//! A small, dependency-free syntax highlighter for diffs.
//!
//! It colours comments, strings, numbers, keywords, type-like names and braces
//! one line at a time. That is deliberately approximate (no multi-line
//! constructs): diff lines are not contiguous, so only per-line rules are safe.

use eframe::egui::{text::LayoutJob, Color32, FontId, TextFormat};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Style {
    Plain,
    Keyword,
    Type,
    Str,
    Comment,
    Number,
    Brace,
}

/// Languages we know a line-comment marker for; anything else is shown plain.
fn comment_marker(ext: &str) -> Option<&'static str> {
    match ext {
        "rs" | "js" | "jsx" | "ts" | "tsx" | "mjs" | "java" | "kt" | "kts" | "c" | "h" | "cc" | "cpp"
        | "hpp" | "cs" | "go" | "swift" | "scala" | "gradle" | "groovy" | "php" | "dart" | "css"
        | "scss" | "json5" | "zig" => Some("//"),
        "py" | "sh" | "bash" | "zsh" | "rb" | "yml" | "yaml" | "toml" | "conf" | "properties" | "pl"
        | "r" | "ps1" | "dockerfile" | "gitignore" | "gitattributes" | "mk" | "cmake" | "tf" => Some("#"),
        "sql" | "lua" | "hs" | "elm" => Some("--"),
        "json" | "xml" | "html" | "htm" | "vue" | "svelte" | "md" | "txt" | "csv" | "lock" => Some(""),
        _ => None,
    }
}

const KEYWORDS: &[&str] = &[
    "abstract", "as", "async", "await", "break", "case", "catch", "class", "const", "continue", "crate",
    "data", "def", "default", "defer", "do", "else", "enum", "export", "extends", "extern", "false",
    "final", "finally", "fn", "for", "from", "fun", "func", "function", "go", "if", "impl", "implements",
    "import", "in", "instanceof", "interface", "is", "let", "match", "mod", "mut", "namespace", "new",
    "null", "nil", "object", "of", "override", "package", "plugins", "private", "protected", "pub",
    "public", "return", "self", "static", "struct", "super", "switch", "this", "throw", "throws",
    "trait", "true", "try", "type", "typealias", "use", "val", "var", "void", "when", "where", "while",
    "with", "yield", "int", "long", "float", "double", "bool", "boolean", "char", "string", "byte",
    "short", "unsigned", "signed", "None", "True", "False", "elif", "except", "lambda", "pass",
    "raise", "global", "echo", "fi", "then", "esac", "done",
];

fn palette(dark: bool, s: Style, plain: Color32) -> Color32 {
    let (d, l) = match s {
        Style::Plain => return plain,
        Style::Keyword => ((0x56, 0x9c, 0xd6), (0x00, 0x00, 0xff)),
        Style::Type => ((0x4e, 0xc9, 0xb0), (0x26, 0x7f, 0x99)),
        Style::Str => ((0xce, 0x91, 0x78), (0xa3, 0x15, 0x15)),
        Style::Comment => ((0x6a, 0x99, 0x55), (0x00, 0x80, 0x00)),
        Style::Number => ((0xb5, 0xce, 0xa8), (0x09, 0x86, 0x58)),
        Style::Brace => ((0xff, 0xd7, 0x00), (0x79, 0x5e, 0x26)),
    };
    let (r, g, b) = if dark { d } else { l };
    Color32::from_rgb(r, g, b)
}

fn classify(text: &str, marker: &str) -> Vec<(Style, String)> {
    let chars: Vec<char> = text.chars().collect();
    let mut out: Vec<(Style, String)> = Vec::new();
    let mut push = |s: Style, t: String| match out.last_mut() {
        Some((ls, lt)) if *ls == s => lt.push_str(&t),
        _ => out.push((s, t)),
    };
    let marker: Vec<char> = marker.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let c = chars[i];
        if !marker.is_empty() && chars[i..].starts_with(&marker) {
            push(Style::Comment, chars[i..].iter().collect());
            break;
        }
        if c == '"' || c == '\'' || c == '`' {
            let mut j = i + 1;
            while j < chars.len() && chars[j] != c {
                if chars[j] == '\\' {
                    j += 1;
                }
                j += 1;
            }
            let end = (j + 1).min(chars.len());
            push(Style::Str, chars[i..end].iter().collect());
            i = end;
        } else if c.is_ascii_digit() {
            let mut j = i;
            while j < chars.len() && (chars[j].is_ascii_alphanumeric() || chars[j] == '.' || chars[j] == '_') {
                j += 1;
            }
            push(Style::Number, chars[i..j].iter().collect());
            i = j;
        } else if c.is_alphabetic() || c == '_' {
            let mut j = i;
            while j < chars.len() && (chars[j].is_alphanumeric() || chars[j] == '_') {
                j += 1;
            }
            let word: String = chars[i..j].iter().collect();
            let style = if KEYWORDS.contains(&word.as_str()) {
                Style::Keyword
            } else if c.is_uppercase() && word.len() > 1 && word.chars().any(|c| c.is_lowercase()) {
                Style::Type
            } else {
                Style::Plain
            };
            push(style, word);
            i = j;
        } else if matches!(c, '{' | '}') {
            push(Style::Brace, c.to_string());
            i += 1;
        } else {
            push(Style::Plain, c.to_string());
            i += 1;
        }
    }
    out
}

/// Lay out one line of code with syntax colours. Files of unknown type are
/// shown in the plain text colour.
pub fn highlight(path: &str, text: &str, plain: Color32, font: &FontId, dark: bool) -> LayoutJob {
    let ext = path.rsplit_once('.').map_or(path.rsplit('/').next().unwrap_or(""), |(_, e)| e).to_lowercase();
    let mut job = LayoutJob::default();
    job.wrap.max_width = f32::INFINITY;
    match comment_marker(&ext) {
        None => {
            job.append(text, 0.0, TextFormat::simple(font.clone(), plain));
        }
        Some(marker) => {
            // Markup/data files: strings and numbers only make sense for code, so
            // colour nothing for those (empty marker means "known, but plain").
            if marker.is_empty() {
                job.append(text, 0.0, TextFormat::simple(font.clone(), plain));
            } else {
                for (style, t) in classify(text, marker) {
                    job.append(&t, 0.0, TextFormat::simple(font.clone(), palette(dark, style, plain)));
                }
            }
        }
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    fn styles(text: &str, marker: &str) -> Vec<(Style, String)> {
        classify(text, marker)
    }

    #[test]
    fn classifies_tokens() {
        let s = styles("import java.time.Instant // note", "//");
        assert_eq!(s[0], (Style::Keyword, "import".into()));
        assert!(s.contains(&(Style::Type, "Instant".into())));
        assert_eq!(s.last().unwrap(), &(Style::Comment, "// note".into()));
    }

    #[test]
    fn strings_numbers_and_braces() {
        let s = styles(r#"id("a//b") version(4.0.0) {"#, "//");
        assert!(s.contains(&(Style::Str, "\"a//b\"".into())), "comment marker inside a string is not a comment");
        assert!(s.contains(&(Style::Number, "4.0.0".into())));
        assert!(s.contains(&(Style::Brace, "{".into())));
    }

    #[test]
    fn unterminated_string_and_hash_comments() {
        let s = styles("x = 'oops", "#");
        assert_eq!(s.last().unwrap().0, Style::Str);
        let s = styles("run # trailing", "#");
        assert_eq!(s.last().unwrap(), &(Style::Comment, "# trailing".into()));
    }
}
