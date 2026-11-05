//! Turns drawing primitives into pixels on screen (egui) or into SVG / PNG / HTML files.

use crate::draw::{self, frame_extent, frame_style, Env};
use crate::model::Project;
use crate::pen::{approx_measure, Pen, Prim, Rgba, R};
use eframe::egui::{self, Color32, FontFamily, FontId, Pos2, Stroke};
use std::sync::OnceLock;

// ---------------------------------------------------------------- fonts

#[derive(Default, Clone, Copy)]
pub struct FontFlags {
    pub hand: bool,
    pub hand_bold: bool,
    pub clean: bool,
    pub clean_bold: bool,
}

static FONTS: OnceLock<FontFlags> = OnceLock::new();

fn first_existing(paths: &[&str]) -> Option<Vec<u8>> {
    paths.iter().find_map(|p| std::fs::read(p).ok())
}

/// Loads a hand-drawn looking font and a clean one from the system, if present.
pub fn install_fonts(ctx: &egui::Context) {
    let mut defs = egui::FontDefinitions::default();
    let mut flags = FontFlags::default();
    let sets: [(&str, &[&str], &mut bool); 4] = [
        (
            "hand",
            &[
                "/System/Library/Fonts/Supplemental/Comic Sans MS.ttf",
                "C:\\Windows\\Fonts\\comic.ttf",
                "/usr/share/fonts/truetype/msttcorefonts/Comic_Sans_MS.ttf",
                "/usr/share/fonts/truetype/comic-neue/ComicNeue-Regular.ttf",
                "/usr/share/fonts/opentype/comic-neue/ComicNeue-Regular.otf",
                "/usr/share/fonts/TTF/ComicNeue-Regular.ttf",
            ],
            &mut flags.hand,
        ),
        (
            "hand_bold",
            &[
                "/System/Library/Fonts/Supplemental/Comic Sans MS Bold.ttf",
                "C:\\Windows\\Fonts\\comicbd.ttf",
                "/usr/share/fonts/truetype/msttcorefonts/Comic_Sans_MS_Bold.ttf",
                "/usr/share/fonts/truetype/comic-neue/ComicNeue-Bold.ttf",
                "/usr/share/fonts/opentype/comic-neue/ComicNeue-Bold.otf",
                "/usr/share/fonts/TTF/ComicNeue-Bold.ttf",
            ],
            &mut flags.hand_bold,
        ),
        (
            "clean",
            &[
                "/System/Library/Fonts/Supplemental/Arial.ttf",
                "C:\\Windows\\Fonts\\arial.ttf",
                "/usr/share/fonts/truetype/liberation/LiberationSans-Regular.ttf",
                "/usr/share/fonts/truetype/dejavu/DejaVuSans.ttf",
            ],
            &mut flags.clean,
        ),
        (
            "clean_bold",
            &[
                "/System/Library/Fonts/Supplemental/Arial Bold.ttf",
                "C:\\Windows\\Fonts\\arialbd.ttf",
                "/usr/share/fonts/truetype/liberation/LiberationSans-Bold.ttf",
                "/usr/share/fonts/truetype/dejavu/DejaVuSans-Bold.ttf",
            ],
            &mut flags.clean_bold,
        ),
    ];
    for (name, paths, flag) in sets {
        if let Some(bytes) = first_existing(paths) {
            defs.font_data.insert(name.to_string(), std::sync::Arc::new(egui::FontData::from_owned(bytes)));
            let mut fam = vec![name.to_string()];
            fam.extend(defs.families.get(&FontFamily::Proportional).cloned().unwrap_or_default());
            defs.families.insert(FontFamily::Name(name.into()), fam);
            *flag = true;
        }
    }
    let _ = FONTS.set(flags);
    ctx.set_fonts(defs);
}

fn flags() -> FontFlags {
    FONTS.get().copied().unwrap_or_default()
}

/// The font family for a piece of text, and whether bold must be faked.
pub fn family(sketch: bool, bold: bool, mono: bool) -> (FontFamily, bool) {
    let f = flags();
    if mono {
        return (FontFamily::Monospace, bold);
    }
    let (reg, bld, has_reg, has_bold) = if sketch { ("hand", "hand_bold", f.hand, f.hand_bold) } else { ("clean", "clean_bold", f.clean, f.clean_bold) };
    if bold && has_bold {
        (FontFamily::Name(bld.into()), false)
    } else if has_reg {
        (FontFamily::Name(reg.into()), bold)
    } else {
        (FontFamily::Proportional, bold)
    }
}

pub fn egui_measure<'a>(ctx: &'a egui::Context, sketch: bool) -> impl Fn(&str, f32, bool) -> f32 + 'a {
    move |s: &str, size: f32, bold: bool| {
        let (fam, fake) = family(sketch, bold, false);
        let w = ctx.fonts_mut(|f| f.layout_no_wrap(s.to_string(), FontId::new(size.max(1.0), fam), Color32::WHITE).size().x);
        w + if fake { s.len() as f32 * 0.04 * size / 10.0 } else { 0.0 }
    }
}

fn c32(c: Rgba) -> Color32 {
    Color32::from_rgba_unmultiplied(c[0], c[1], c[2], c[3])
}

// ---------------------------------------------------------------- egui

pub fn paint(painter: &egui::Painter, prims: &[Prim], origin: Pos2, zoom: f32, sketch: bool) {
    let tp = |p: &[f32; 2]| Pos2::new(origin.x + p[0] * zoom, origin.y + p[1] * zoom);
    for prim in prims {
        match prim {
            Prim::Path { pts, closed, fill, stroke, convex } => {
                if pts.len() < 2 {
                    continue;
                }
                let v: Vec<Pos2> = pts.iter().map(tp).collect();
                if let Some(f) = fill {
                    if v.len() >= 3 {
                        if *convex {
                            painter.add(egui::Shape::convex_polygon(v.clone(), c32(*f), Stroke::NONE));
                        } else {
                            let mut mesh = egui::Mesh::default();
                            let col = c32(*f);
                            for p in &v {
                                mesh.colored_vertex(*p, col);
                            }
                            for i in 1..v.len() as u32 - 1 {
                                mesh.add_triangle(0, i, i + 1);
                            }
                            painter.add(egui::Shape::mesh(mesh));
                        }
                    }
                }
                if let Some((w, c)) = stroke {
                    let st = Stroke::new((w * zoom).max(0.6), c32(*c));
                    if *closed {
                        painter.add(egui::Shape::closed_line(v, st));
                    } else {
                        painter.add(egui::Shape::line(v, st));
                    }
                }
            }
            Prim::Text { x, y, s, size, color, bold, italic: _, underline, mono, hx } => {
                let px = size * zoom;
                if px < 2.5 {
                    // too small to read: a grey bar stands in for it
                    let w = approx_measure(s, *size, *bold) * zoom;
                    let x0 = origin.x + x * zoom - w * hx;
                    let yy = origin.y + y * zoom;
                    painter.line_segment([Pos2::new(x0, yy), Pos2::new(x0 + w, yy)], Stroke::new((px * 0.5).max(0.5), c32(*color).gamma_multiply(0.5)));
                    continue;
                }
                let (fam, fake) = family(sketch, *bold, *mono);
                let galley = painter.layout_no_wrap(s.clone(), FontId::new(px, fam), c32(*color));
                let size_v = galley.size();
                let pos = Pos2::new(origin.x + x * zoom - size_v.x * hx, origin.y + y * zoom - size_v.y / 2.0);
                if fake {
                    painter.galley(pos + egui::vec2((px * 0.04).max(0.5), 0.0), galley.clone(), c32(*color));
                }
                painter.galley(pos, galley, c32(*color));
                if *underline {
                    let yy = pos.y + size_v.y * 0.85;
                    painter.line_segment([Pos2::new(pos.x, yy), Pos2::new(pos.x + size_v.x, yy)], Stroke::new((px / 14.0).max(0.8), c32(*color)));
                }
            }
            Prim::Image { r, img, fit, alpha } => {
                let (dst, uv) = crate::images::fit_rect(*r, img.w, img.h, *fit);
                let rect = egui::Rect::from_min_size(tp(&[dst.x, dst.y]), egui::vec2(dst.w * zoom, dst.h * zoom));
                let px = dst.w * zoom * painter.ctx().pixels_per_point();
                if let Some(tex) = crate::images::texture(painter.ctx(), img, px) {
                    painter.image(tex, rect, uv, Color32::WHITE.gamma_multiply(*alpha));
                } else {
                    painter.rect_filled(rect, 0.0, Color32::from_gray(225));
                }
            }
            Prim::Link { .. } => {}
        }
    }
}

// ---------------------------------------------------------------- SVG

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

fn rgb(c: Rgba) -> String {
    format!("rgb({},{},{})", c[0], c[1], c[2])
}

pub fn svg_body(prims: &[Prim], sketch: bool, out: &mut String, links: bool) {
    let hand = "'Comic Sans MS','Comic Neue','Chalkboard SE','Segoe Print',cursive";
    let clean = "Arial,'Liberation Sans','Helvetica Neue',Helvetica,sans-serif";
    for p in prims {
        match p {
            Prim::Path { pts, closed, fill, stroke, .. } => {
                if pts.len() < 2 {
                    continue;
                }
                let mut d = String::with_capacity(pts.len() * 14);
                for (i, q) in pts.iter().enumerate() {
                    d.push_str(&format!("{}{:.1} {:.1}", if i == 0 { "M" } else { " L" }, q[0], q[1]));
                }
                if let Some(f) = fill {
                    out.push_str(&format!("<path d=\"{d} Z\" fill=\"{}\" fill-opacity=\"{:.3}\"/>\n", rgb(*f), f[3] as f32 / 255.0));
                }
                if let Some((w, c)) = stroke {
                    let z = if *closed { " Z" } else { "" };
                    out.push_str(&format!(
                        "<path d=\"{d}{z}\" fill=\"none\" stroke=\"{}\" stroke-opacity=\"{:.3}\" stroke-width=\"{w:.2}\" stroke-linejoin=\"round\" stroke-linecap=\"round\"/>\n",
                        rgb(*c),
                        c[3] as f32 / 255.0
                    ));
                }
            }
            Prim::Text { x, y, s, size, color, bold, italic, underline, mono, hx } => {
                let anchor = if *hx < 0.25 { "start" } else if *hx > 0.75 { "end" } else { "middle" };
                let fam = if *mono { "Menlo,Consolas,'DejaVu Sans Mono',monospace" } else if sketch { hand } else { clean };
                out.push_str(&format!(
                    "<text x=\"{x:.1}\" y=\"{:.1}\" font-size=\"{size:.1}\" font-family=\"{fam}\" text-anchor=\"{anchor}\" fill=\"{}\" fill-opacity=\"{:.3}\"{}{}{} xml:space=\"preserve\">{}</text>\n",
                    y + size * 0.36,
                    rgb(*color),
                    color[3] as f32 / 255.0,
                    if *bold { " font-weight=\"bold\"" } else { "" },
                    if *italic { " font-style=\"italic\"" } else { "" },
                    if *underline { " text-decoration=\"underline\"" } else { "" },
                    esc(s)
                ));
            }
            Prim::Image { r, img, fit, alpha } => {
                out.push_str(&format!(
                    "<image x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" preserveAspectRatio=\"{}\" opacity=\"{:.3}\" href=\"{}\"/>\n",
                    r.x,
                    r.y,
                    r.w,
                    r.h,
                    fit.svg_aspect(),
                    alpha,
                    img.data_uri()
                ));
            }
            Prim::Link { r, target } => {
                if links {
                    out.push_str(&format!(
                        "<a href=\"#{}\"><rect class=\"hot\" x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" rx=\"4\"/></a>\n",
                        esc(target),
                        r.x,
                        r.y,
                        r.w,
                        r.h
                    ));
                }
            }
        }
    }
}

/// The area a screen takes including its device frame.
pub fn screen_bounds(project: &Project, idx: usize, with_frame: bool) -> R {
    let s = &project.screens[idx];
    let (l, t, r, b) = if with_frame { frame_extent(frame_style(&s.device)) } else { (0.0, 0.0, 0.0, 0.0) };
    let pad = if with_frame { 24.0 } else { 0.0 };
    R::new(-l - pad, -t - pad, s.w + l + r + pad * 2.0, s.h + t + b + pad * 2.0)
}

pub fn screen_prims(project: &Project, idx: usize, sketch: bool, with_frame: bool) -> Vec<Prim> {
    let m = approx_measure;
    let mut pen = Pen::new(sketch, false, &m);
    let env = Env { project, depth: 0 };
    draw::screen(&mut pen, &env, idx, with_frame);
    pen.out
}

pub fn screen_svg(project: &Project, idx: usize, sketch: bool, with_frame: bool) -> String {
    let b = screen_bounds(project, idx, with_frame);
    let prims = screen_prims(project, idx, sketch, with_frame);
    let mut s = format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{:.0}\" height=\"{:.0}\" viewBox=\"{:.1} {:.1} {:.1} {:.1}\">\n",
        b.w, b.h, b.x, b.y, b.w, b.h
    );
    if with_frame {
        s.push_str(&format!("<rect x=\"{:.1}\" y=\"{:.1}\" width=\"{:.1}\" height=\"{:.1}\" fill=\"#f2f2f0\"/>\n", b.x, b.y, b.w, b.h));
    }
    svg_body(&prims, sketch, &mut s, false);
    s.push_str("</svg>\n");
    s
}

static FONTDB: OnceLock<std::sync::Arc<resvg::usvg::fontdb::Database>> = OnceLock::new();

/// System fonts for drawing SVG text (loaded once).
pub fn fontdb() -> std::sync::Arc<resvg::usvg::fontdb::Database> {
    FONTDB
        .get_or_init(|| {
            let mut db = resvg::usvg::fontdb::Database::new();
            db.load_system_fonts();
            db.set_sans_serif_family("Arial");
            std::sync::Arc::new(db)
        })
        .clone()
}

pub fn svg_to_png(svg: &str, scale: f32) -> Result<Vec<u8>, String> {
    use resvg::{tiny_skia, usvg};
    let mut opt = usvg::Options::default();
    opt.fontdb = fontdb();
    let tree = usvg::Tree::from_str(svg, &opt).map_err(|e| e.to_string())?;
    let size = tree.size();
    let w = ((size.width() * scale).ceil() as u32).clamp(1, 8000);
    let h = ((size.height() * scale).ceil() as u32).clamp(1, 8000);
    let mut pixmap = tiny_skia::Pixmap::new(w, h).ok_or("picture too large")?;
    pixmap.fill(tiny_skia::Color::WHITE);
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    pixmap.encode_png().map_err(|e| e.to_string())
}

pub fn screen_png(project: &Project, idx: usize, sketch: bool, with_frame: bool, scale: f32) -> Result<Vec<u8>, String> {
    svg_to_png(&screen_svg(project, idx, sketch, with_frame), scale)
}

/// One HTML file: every screen, clickable links between them, notes underneath.
pub fn prototype_html(project: &Project, sketch: bool) -> String {
    let mut h = String::new();
    h.push_str("<!doctype html><html><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">");
    h.push_str(&format!("<title>{}</title>", esc(&project.name)));
    h.push_str(
        "<style>body{margin:0;background:#ecebe8;font-family:system-ui,sans-serif;color:#222}\
nav{position:sticky;top:0;background:#fff;border-bottom:1px solid #ccc;padding:8px 16px;display:flex;gap:6px;flex-wrap:wrap;z-index:2}\
nav a{padding:6px 12px;border-radius:16px;text-decoration:none;color:#222;background:#f2f2f2}\
nav a.on{background:#4076cc;color:#fff}\
section{display:none;padding:24px;text-align:center}section.on{display:block}\
svg{max-width:100%;height:auto;filter:drop-shadow(0 4px 14px rgba(0,0,0,.12))}\
.hot{fill:#4076cc;fill-opacity:0;cursor:pointer;transition:fill-opacity .15s}.hot:hover{fill-opacity:.18}\
.notes{max-width:720px;margin:16px auto;text-align:left;white-space:pre-wrap;background:#fff8c4;padding:12px 16px;border-radius:6px}\
</style></head><body><nav>",
    );
    for s in project.screens.iter().filter(|s| !s.is_component) {
        h.push_str(&format!("<a href=\"#{}\">{}</a>", esc(&s.id), esc(&s.name)));
    }
    h.push_str("</nav>\n");
    for (i, s) in project.screens.iter().enumerate() {
        if s.is_component {
            continue;
        }
        let b = screen_bounds(project, i, true);
        let prims = screen_prims(project, i, sketch, true);
        h.push_str(&format!(
            "<section id=\"{}\"><svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{:.0}\" height=\"{:.0}\" viewBox=\"{:.1} {:.1} {:.1} {:.1}\">\n",
            esc(&s.id),
            b.w,
            b.h,
            b.x,
            b.y,
            b.w,
            b.h
        ));
        svg_body(&prims, sketch, &mut h, true);
        h.push_str("</svg>");
        if !s.notes.is_empty() {
            h.push_str(&format!("<div class=\"notes\">{}</div>", esc(&s.notes)));
        }
        h.push_str("</section>\n");
    }
    h.push_str(
        "<script>function show(){var id=location.hash.slice(1);var ss=document.querySelectorAll('section');\
if(!document.getElementById(id)&&ss.length){id=ss[0].id}\
ss.forEach(function(s){s.classList.toggle('on',s.id===id)});\
document.querySelectorAll('nav a').forEach(function(a){a.classList.toggle('on',a.getAttribute('href')==='#'+id)});\
window.scrollTo(0,0)}window.addEventListener('hashchange',show);show();</script></body></html>\n",
    );
    h
}
