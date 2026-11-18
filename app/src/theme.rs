//! Оформление, перенесённое из оригинального style.css: палитра, стеклянные панели, градиенты, фон, шрифты.
//! Все ассеты вшиты в exe (include_bytes!), поэтому работают одинаково из portable и из установленной папки.

use eframe::egui::{self, Color32, FontData, FontDefinitions, FontFamily, Rounding, Stroke, TextureHandle, TextureOptions};

pub const LOGO_PNG: &[u8] = include_bytes!("../../assets/logo.png");
pub const ICON_PNG: &[u8] = include_bytes!("../../assets/icon.png");
pub const STEVE_PNG: &[u8] = include_bytes!("../../assets/steve.png");

// :root из style.css, акцент "redshift" (#ff2d3a → #ff7a3d)
pub const A1: Color32 = Color32::from_rgb(0xff, 0x2d, 0x3a);
pub const A2: Color32 = Color32::from_rgb(0xff, 0x7a, 0x3d);
pub const BG0: Color32 = Color32::from_rgb(0x07, 0x0a, 0x12);
pub const BG1: Color32 = Color32::from_rgb(0x0d, 0x12, 0x20);
pub const TEXT: Color32 = Color32::from_rgb(0xee, 0xf2, 0xff);
pub const MUTE: Color32 = Color32::from_rgb(0x8f, 0x9a, 0xb5);
pub const DANGER: Color32 = Color32::from_rgb(0xff, 0x6b, 0x6b);
pub const RADIUS: f32 = 16.0;

/// Цвет с альфой из CSS rgba(): премультипликация считается напрямую (как в браузере), без гамма-коррекции egui.
pub fn ca(r: u8, g: u8, b: u8, a: u8) -> Color32 {
    let m = |c: u8| ((c as u32 * a as u32 + 127) / 255) as u8;
    Color32::from_rgba_premultiplied(m(r), m(g), m(b), a)
}
pub fn wa(a: u8) -> Color32 { ca(255, 255, 255, a) }

pub fn glass_fill() -> Color32 { wa(12) }       // rgba(255,255,255,.045)
pub fn glass_fill2() -> Color32 { wa(19) }      // .075
pub fn line() -> Color32 { wa(23) }             // .09
pub fn line2() -> Color32 { wa(41) }            // .16

pub fn decode(png: &[u8]) -> egui::ColorImage {
    let im = image::load_from_memory_with_format(png, image::ImageFormat::Png).expect("встроенный PNG").to_rgba8();
    let size = [im.width() as usize, im.height() as usize];
    egui::ColorImage::from_rgba_unmultiplied(size, im.as_raw())
}

pub fn window_icon() -> egui::IconData {
    let im = image::load_from_memory_with_format(ICON_PNG, image::ImageFormat::Png).expect("icon").to_rgba8();
    egui::IconData { width: im.width(), height: im.height(), rgba: im.into_raw() }
}

pub struct Textures { pub logo: TextureHandle, pub steve: TextureHandle, pub bg: TextureHandle, pub grad: TextureHandle }

fn mix(a: Color32, b: Color32, t: f32) -> [f32; 3] {
    [a.r() as f32 * (1.0 - t) + b.r() as f32 * t, a.g() as f32 * (1.0 - t) + b.g() as f32 * t, a.b() as f32 * (1.0 - t) + b.b() as f32 * t]
}

/// Статичный фон: линейный градиент 160° + два радиальных свечения + три «блоба» из .bg/.blob.
/// Рисуется один раз в текстуру → в простое стоит 0 CPU (в оригинале блобы анимировались с blur(90px)).
fn background_image() -> egui::ColorImage {
    let (w, h) = (320usize, 200usize);
    let mut px = vec![Color32::BLACK; w * h];
    let glow = |x: f32, y: f32, cx: f32, cy: f32, rx: f32, ry: f32| -> f32 {
        let d = (((x - cx) / rx).powi(2) + ((y - cy) / ry).powi(2)).sqrt();
        (1.0 - d).clamp(0.0, 1.0).powi(2)
    };
    for j in 0..h {
        for i in 0..w {
            let (x, y) = (i as f32 / w as f32, j as f32 / h as f32);
            let t = ((x * 0.34 + y * 0.94) / 1.28).clamp(0.0, 1.0); // направление 160deg
            let mut c = mix(BG1, BG0, t);
            let mut add = |col: Color32, k: f32| { for (n, v) in [col.r(), col.g(), col.b()].into_iter().enumerate() { c[n] = c[n] * (1.0 - k) + v as f32 * k; } };
            add(A2, 0.18 * glow(x, y, 0.80, -0.10, 1.1, 1.0));
            add(A1, 0.16 * glow(x, y, -0.10, 1.10, 0.8, 0.9));
            add(A1, 0.38 * glow(x, y, 0.05, 0.10, 0.42, 0.55));   // .b1
            add(A2, 0.38 * glow(x, y, 0.97, 0.45, 0.38, 0.55));   // .b2
            let m = if true { Color32::from_rgb(((A1.r() as u16 + A2.r() as u16) / 2) as u8, ((A1.g() as u16 + A2.g() as u16) / 2) as u8, ((A1.b() as u16 + A2.b() as u16) / 2) as u8) } else { A1 };
            add(m, 0.30 * glow(x, y, 0.45, 1.05, 0.36, 0.45));    // .b3
            px[j * w + i] = Color32::from_rgb(c[0] as u8, c[1] as u8, c[2] as u8);
        }
    }
    egui::ColorImage { size: [w, h], pixels: px }
}

fn gradient_image() -> egui::ColorImage {
    let w = 256usize;
    egui::ColorImage { size: [w, 1], pixels: (0..w).map(|i| { let c = mix(A1, A2, i as f32 / (w - 1) as f32); Color32::from_rgb(c[0] as u8, c[1] as u8, c[2] as u8) }).collect() }
}

pub fn load_textures(ctx: &egui::Context) -> Textures {
    Textures {
        logo: ctx.load_texture("logo", decode(LOGO_PNG), TextureOptions::LINEAR),
        steve: ctx.load_texture("steve", decode(STEVE_PNG), TextureOptions::NEAREST),
        bg: ctx.load_texture("bg", background_image(), TextureOptions::LINEAR),
        grad: ctx.load_texture("grad", gradient_image(), TextureOptions::LINEAR),
    }
}

/// Шрифт оригинала — Segoe UI (системный на Windows). Если его нет, остаётся встроенный шрифт egui.
pub fn install_fonts(ctx: &egui::Context) -> Option<&'static str> {
    let candidates = ["C:\\Windows\\Fonts\\segoeuivf.ttf", "C:\\Windows\\Fonts\\segoeui.ttf"];
    for p in candidates {
        if let Ok(bytes) = std::fs::read(p) {
            let mut fonts = FontDefinitions::default();
            fonts.font_data.insert("segoe".into(), FontData::from_owned(bytes));
            fonts.families.get_mut(&FontFamily::Proportional).unwrap().insert(0, "segoe".into());
            ctx.set_fonts(fonts);
            return Some(p);
        }
    }
    None
}

pub fn apply_style(ctx: &egui::Context) {
    let mut v = egui::Visuals::dark();
    v.panel_fill = Color32::TRANSPARENT; v.window_fill = Color32::from_rgb(0x10, 0x15, 0x24); v.extreme_bg_color = ca(0, 0, 0, 70);
    v.override_text_color = Some(TEXT);
    v.selection.bg_fill = ca(0xff, 0x2d, 0x3a, 110); v.selection.stroke = Stroke::new(1.0, A1);
    v.hyperlink_color = A2;
    v.window_rounding = Rounding::same(RADIUS); v.window_stroke = Stroke::new(1.0, line2());
    v.menu_rounding = Rounding::same(12.0);
    for w in [&mut v.widgets.inactive, &mut v.widgets.hovered, &mut v.widgets.active, &mut v.widgets.noninteractive, &mut v.widgets.open] { w.rounding = Rounding::same(12.0); }
    v.widgets.inactive.bg_fill = glass_fill2(); v.widgets.inactive.weak_bg_fill = glass_fill2(); v.widgets.inactive.bg_stroke = Stroke::new(1.0, line());
    v.widgets.hovered.bg_fill = wa(30); v.widgets.hovered.weak_bg_fill = wa(30); v.widgets.hovered.bg_stroke = Stroke::new(1.0, line2());
    v.widgets.active.bg_fill = ca(0xff, 0x2d, 0x3a, 90); v.widgets.active.weak_bg_fill = v.widgets.active.bg_fill;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, line());
    ctx.set_visuals(v);
    let mut s = (*ctx.style()).clone();
    s.spacing.item_spacing = egui::vec2(10.0, 10.0); s.spacing.button_padding = egui::vec2(14.0, 8.0);
    s.interaction.selectable_labels = false;
    ctx.set_style(s);
}
