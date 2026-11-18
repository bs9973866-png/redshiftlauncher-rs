//! Компоненты в стиле оригинального style.css: стекло, градиентные кнопки, навигация, иконки.

use crate::theme::*;
use eframe::egui::{self, pos2, vec2, Color32, Painter, Pos2, Rect, Response, RichText, Rounding, Sense, Stroke, TextureHandle};

pub fn glass<R>(ui: &mut egui::Ui, pad: f32, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    let r = egui::Frame::none().fill(glass_fill()).stroke(Stroke::new(1.0, line())).rounding(RADIUS).inner_margin(pad)
        .shadow(egui::epaint::Shadow { offset: vec2(0.0, 10.0), blur: 30.0, spread: 0.0, color: Color32::from_black_alpha(50) })
        .show(ui, |ui| { ui.set_width(ui.available_width()); add(ui) });
    ui.add_space(10.0);
    r.inner
}

pub fn title(ui: &mut egui::Ui, h: &str, sub: &str) {
    ui.add_space(6.0);
    ui.label(RichText::new(h).size(30.0).strong());
    ui.label(RichText::new(sub).color(MUTE));
    ui.add_space(12.0);
}

/// Мягкое свечение вокруг прямоугольника (аналог drop-shadow с цветом акцента).
pub fn glow_rect(p: &Painter, r: Rect, rad: f32, col: Color32) {
    for (g, a) in [(10.0, 6u8), (6.0, 9), (3.0, 12)] { p.rect_filled(r.expand(g), rad + g, ca(col.r(), col.g(), col.b(), a)); }
}

pub fn grad_button(ui: &mut egui::Ui, tex: &crate::theme::Textures, text: &str, size: egui::Vec2, enabled: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(size, if enabled { Sense::click() } else { Sense::hover() });
    let hover = enabled && resp.hovered();
    let r = rect.shrink(if resp.is_pointer_button_down_on() && enabled { 1.5 } else { 0.0 });
    let tint = if enabled { if hover { Color32::WHITE } else { Color32::from_gray(235) } } else { wa(60) };
    if enabled { glow_rect(ui.painter(), r, 14.0, if hover { A1 } else { A2 }); }
    egui::Image::new(egui::load::SizedTexture::new(tex.grad.id(), r.size())).rounding(14.0).tint(tint).paint_at(ui, r);
    ui.painter().text(r.center(), egui::Align2::CENTER_CENTER, text, egui::FontId::proportional(if size.y > 50.0 { 17.0 } else { 14.0 }), Color32::from_rgb(0x1a, 0x04, 0x06));
    if enabled { resp.on_hover_cursor(egui::CursorIcon::PointingHand) } else { resp }
}

pub fn soft_button(ui: &mut egui::Ui, text: &str, danger: bool, enabled: bool) -> Response {
    let col = if danger { DANGER } else { TEXT };
    let b = egui::Button::new(RichText::new(text).strong().size(13.0).color(if enabled { col } else { MUTE }))
        .fill(if danger { ca(255, 107, 107, 22) } else { glass_fill2() })
        .stroke(Stroke::new(1.0, if danger { ca(255, 107, 107, 60) } else { line2() })).rounding(12.0);
    ui.add_enabled(enabled, b)
}

#[derive(Clone, Copy)] pub enum Icon { Play, Cube, Person, Users, Sliders }

fn draw_icon(p: &Painter, ic: Icon, c: Pos2, col: Color32) {
    let s = Stroke::new(1.7, col);
    match ic {
        Icon::Play => { p.add(egui::Shape::closed_line(vec![c + vec2(-4.0, -7.0), c + vec2(6.0, 0.0), c + vec2(-4.0, 7.0)], s)); }
        Icon::Cube => { let h = vec![c + vec2(0.0, -8.0), c + vec2(7.5, -4.0), c + vec2(7.5, 4.0), c + vec2(0.0, 8.0), c + vec2(-7.5, 4.0), c + vec2(-7.5, -4.0)];
            p.add(egui::Shape::closed_line(h, s)); p.line_segment([c + vec2(-7.5, -4.0), c], s); p.line_segment([c + vec2(7.5, -4.0), c], s); p.line_segment([c, c + vec2(0.0, 8.0)], s); }
        Icon::Person => { p.circle_stroke(c + vec2(0.0, -3.5), 4.0, s); p.add(egui::Shape::line(vec![c + vec2(-7.0, 8.0), c + vec2(-6.0, 3.5), c + vec2(6.0, 3.5), c + vec2(7.0, 8.0)], s)); }
        Icon::Users => { p.circle_stroke(c + vec2(-3.0, -3.5), 3.6, s); p.add(egui::Shape::line(vec![c + vec2(-9.0, 7.5), c + vec2(-8.0, 3.0), c + vec2(2.0, 3.0), c + vec2(3.0, 7.5)], s));
            p.circle_stroke(c + vec2(5.5, -4.5), 3.0, Stroke::new(1.4, col)); }
        Icon::Sliders => { for (i, k) in [0.3f32, 0.7, 0.45].iter().enumerate() { let y = -6.0 + i as f32 * 6.0; p.line_segment([c + vec2(-8.0, y), c + vec2(8.0, y)], s); p.circle_filled(c + vec2(-8.0 + 16.0 * k, y), 2.4, col); } }
    }
}

pub fn nav_button(ui: &mut egui::Ui, active: bool, text: &str, ic: Icon) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 42.0), Sense::click());
    let p = ui.painter();
    if active {
        glow_rect(p, rect, 12.0, A1);
        p.rect(rect, 12.0, ca(0xff, 0x2d, 0x3a, 40), Stroke::new(1.0, ca(0xff, 0x2d, 0x3a, 110)));
    } else if resp.hovered() { p.rect_filled(rect, 12.0, wa(14)); }
    let col = if active { TEXT } else if resp.hovered() { TEXT } else { MUTE };
    draw_icon(p, ic, pos2(rect.min.x + 26.0, rect.center().y), if active { A2 } else { col });
    p.text(pos2(rect.min.x + 47.0, rect.center().y), egui::Align2::LEFT_CENTER, text, egui::FontId::proportional(14.5), col);
    ui.add_space(-4.0);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn account_chip(ui: &mut egui::Ui, tex: &Textures, name: &str, sub: &str, has: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 62.0), Sense::click());
    let p = ui.painter();
    p.rect(rect, 14.0, if resp.hovered() { wa(22) } else { glass_fill() }, Stroke::new(1.0, line()));
    let av = Rect::from_min_size(rect.min + vec2(12.0, 11.0), vec2(40.0, 40.0));
    if has {
        // голова из skin-атласа: область лица (8,8)-(16,16) + слой волос (40,8)-(48,16)
        let uv = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_max(pos2(x / 64.0, y / 64.0), pos2((x + w) / 64.0, (y + h) / 64.0));
        p.image(tex.steve.id(), av, uv(8.0, 8.0, 8.0, 8.0), Color32::WHITE);
        p.image(tex.steve.id(), av, uv(40.0, 8.0, 8.0, 8.0), Color32::WHITE);
    } else { p.rect_filled(av, 10.0, wa(14)); }
    p.text(pos2(av.max.x + 12.0, rect.center().y - 9.0), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(14.5), TEXT);
    p.text(pos2(av.max.x + 12.0, rect.center().y + 10.0), egui::Align2::LEFT_CENTER, sub, egui::FontId::proportional(11.5), MUTE);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn instance_card(ui: &mut egui::Ui, selected: bool, name: &str, sub: &str) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), 62.0), Sense::click());
    let p = ui.painter();
    let fill = if selected { ca(0xff, 0x2d, 0x3a, 28) } else if resp.hovered() { glass_fill2() } else { glass_fill() };
    p.rect(rect, 14.0, fill, Stroke::new(1.0, if selected { ca(0xff, 0x2d, 0x3a, 120) } else { line() }));
    p.text(rect.min + vec2(20.0, 21.0), egui::Align2::LEFT_CENTER, name, egui::FontId::proportional(15.0), TEXT);
    p.text(rect.min + vec2(20.0, 42.0), egui::Align2::LEFT_CENTER, sub, egui::FontId::proportional(12.0), MUTE);
    ui.add_space(-2.0);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn progress_bar(ui: &mut egui::Ui, tex: &Textures, v: f32, busy: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 10.0), Sense::hover());
    ui.painter().rect_filled(rect, 5.0, wa(18));
    // Индикатор без анимации: при неизвестном прогрессе показываем заполнение 100% тусклым цветом, чтобы не гонять перерисовку.
    let w = if v > 0.0 { v.clamp(0.0, 1.0) } else if busy { 1.0 } else { 0.0 };
    if w > 0.0 {
        let r = Rect::from_min_size(rect.min, vec2(rect.width() * w, rect.height()));
        egui::Image::new(egui::load::SizedTexture::new(tex.grad.id(), r.size())).rounding(5.0).tint(if v > 0.0 { Color32::WHITE } else { wa(90) }).paint_at(ui, r);
    }
}

#[derive(Clone, Copy, PartialEq)] pub enum WinBtn { Min, Max, Close }

pub fn window_button(ui: &mut egui::Ui, rect: Rect, kind: WinBtn) -> Response {
    let resp = ui.interact(rect, ui.id().with(match kind { WinBtn::Min => "min", WinBtn::Max => "max", WinBtn::Close => "close" }), Sense::click());
    let p = ui.painter();
    if resp.hovered() { p.rect_filled(rect, Rounding::ZERO, if kind == WinBtn::Close { Color32::from_rgb(0xe5, 0x48, 0x4d) } else { wa(20) }); }
    let col = if resp.hovered() { Color32::WHITE } else { MUTE }; let s = Stroke::new(1.3, col); let c = rect.center();
    match kind {
        WinBtn::Min => { p.line_segment([c + vec2(-5.0, 0.0), c + vec2(5.0, 0.0)], s); }
        WinBtn::Max => { p.rect_stroke(Rect::from_center_size(c, vec2(10.0, 10.0)), 1.0, s); }
        WinBtn::Close => { p.line_segment([c + vec2(-5.0, -5.0), c + vec2(5.0, 5.0)], s); p.line_segment([c + vec2(-5.0, 5.0), c + vec2(5.0, -5.0)], s); }
    }
    resp
}

/// Плоский вид скина спереди из атласа 64×64 (голова, тело, руки, ноги + внешний слой головы).
pub fn skin_preview(ui: &mut egui::Ui, steve: &TextureHandle, px: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(16.0 * px, 32.0 * px), Sense::hover());
    let uv = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_max(pos2(x / 64.0, y / 64.0), pos2((x + w) / 64.0, (y + h) / 64.0));
    let at = |x: f32, y: f32, w: f32, h: f32| Rect::from_min_size(rect.min + vec2(x * px, y * px), vec2(w * px, h * px));
    let p = ui.painter();
    for (dst, src) in [(at(4.0, 0.0, 8.0, 8.0), uv(8.0, 8.0, 8.0, 8.0)), (at(4.0, 0.0, 8.0, 8.0), uv(40.0, 8.0, 8.0, 8.0)), (at(4.0, 8.0, 8.0, 12.0), uv(20.0, 20.0, 8.0, 12.0)),
                       (at(0.0, 8.0, 4.0, 12.0), uv(44.0, 20.0, 4.0, 12.0)), (at(12.0, 8.0, 4.0, 12.0), uv(36.0, 52.0, 4.0, 12.0)),
                       (at(4.0, 20.0, 4.0, 12.0), uv(4.0, 20.0, 4.0, 12.0)), (at(8.0, 20.0, 4.0, 12.0), uv(20.0, 52.0, 4.0, 12.0))] {
        p.image(steve.id(), dst, src, Color32::WHITE);
    }
}

pub fn open_path(p: &std::path::Path) {
    #[cfg(windows)] { let _ = std::process::Command::new("explorer").arg(p).spawn(); }
    #[cfg(target_os = "macos")] { let _ = std::process::Command::new("open").arg(p).spawn(); }
    #[cfg(all(unix, not(target_os = "macos")))] { let _ = std::process::Command::new("xdg-open").arg(p).spawn(); }
}
