//! Shared look & feel for all app windows: "black + orange".
//!
//! Every value comes from docs/DESIGN.md (Part B). Dark only: there is no
//! light theme and no theme picker. Neutral near-black surfaces in the
//! shadcn/ui manner, one orange accent, and a small kit of components
//! (buttons in five variants, badge, kbd, switch, nav item) so screens never
//! hand-pick a colour or restyle a widget inline.

use eframe::egui::{self, Color32, FontFamily, FontId};

// ---------------------------------------------------------------- palette
// Neutral near-black, no tint. Three surface steps above the window, so depth
// comes from fill + hairline, never from a shadow.

pub const BG: Color32 = Color32::from_rgb(10, 10, 10); // window
pub const SIDEBAR: Color32 = Color32::from_rgb(14, 14, 14); // nav rail
pub const SURFACE: Color32 = Color32::from_rgb(19, 19, 19); // cards
pub const SURFACE_2: Color32 = Color32::from_rgb(26, 26, 26); // inputs, secondary buttons
pub const SURFACE_3: Color32 = Color32::from_rgb(38, 38, 38); // hover / active fills
pub const FG: Color32 = Color32::from_rgb(250, 250, 250); // primary text
pub const TEXT_2: Color32 = Color32::from_rgb(163, 163, 163); // secondary text
pub const MUTED: Color32 = Color32::from_rgb(115, 115, 115); // labels, metadata
/// 1px hairline.
pub const BORDER: Color32 = Color32::from_rgb(36, 36, 36);
/// Hover / focus ring.
pub const RING: Color32 = Color32::from_rgb(60, 60, 60);

/// The one accent: orange-500. Primary buttons, active nav, switches, the
/// recording state, granted permissions, progress.
pub const ACCENT: Color32 = Color32::from_rgb(249, 115, 22);
/// Hovered primary button.
pub const ACCENT_HOVER: Color32 = Color32::from_rgb(251, 140, 60);
/// Text and glyphs on an orange fill.
pub const ON_ACCENT: Color32 = Color32::from_rgb(20, 10, 2);
/// Errors and destructive actions only.
pub const RED: Color32 = Color32::from_rgb(239, 68, 68);
/// Advisories that still work ("note:" problems, the live-typing caveat).
pub const AMBER: Color32 = Color32::from_rgb(245, 158, 11);

/// `color` at ~9% alpha: badge fills behind signal-coloured text.
pub fn tint(color: Color32) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 22)
}

/// `color` at ~18% alpha: the ring around a tinted plate.
pub fn tint_strong(color: Color32) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 46)
}

// ------------------------------------------------------------------ fonts

/// Geist (sans) + Geist Mono, embedded; egui-phosphor appended for icons.
/// Families: `Proportional` → Geist, `Monospace` → Geist Mono, plus named
/// "GeistMedium" / "GeistSemiBold" / "GeistMonoMedium" for emphasis (egui's
/// `strong()` only recolors, so weight needs a family switch).
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let data: [(&str, &[u8]); 5] = [
        ("geist", include_bytes!("../assets/fonts/Geist-Regular.ttf")),
        (
            "geist-medium",
            include_bytes!("../assets/fonts/Geist-Medium.ttf"),
        ),
        (
            "geist-semibold",
            include_bytes!("../assets/fonts/Geist-SemiBold.ttf"),
        ),
        (
            "geist-mono",
            include_bytes!("../assets/fonts/GeistMono-Regular.ttf"),
        ),
        (
            "geist-mono-medium",
            include_bytes!("../assets/fonts/GeistMono-Medium.ttf"),
        ),
    ];
    for (name, bytes) in data {
        fonts.font_data.insert(
            name.to_owned(),
            std::sync::Arc::new(egui::FontData::from_static(bytes)),
        );
    }
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);

    fonts
        .families
        .get_mut(&FontFamily::Proportional)
        .unwrap()
        .insert(0, "geist".to_owned());
    fonts
        .families
        .get_mut(&FontFamily::Monospace)
        .unwrap()
        .insert(0, "geist-mono".to_owned());

    let prop = fonts.families[&FontFamily::Proportional].clone();
    let mono = fonts.families[&FontFamily::Monospace].clone();
    for (family, face, base) in [
        ("GeistMedium", "geist-medium", &prop),
        ("GeistSemiBold", "geist-semibold", &prop),
        ("GeistMonoMedium", "geist-mono-medium", &mono),
    ] {
        let mut chain = base.clone();
        chain.insert(0, face.to_owned());
        fonts
            .families
            .insert(FontFamily::Name(family.into()), chain);
    }
    ctx.set_fonts(fonts);
}

pub fn medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("GeistMedium".into()))
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("GeistSemiBold".into()))
}

pub fn mono_medium(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("GeistMonoMedium".into()))
}

// ------------------------------------------------------------------ style

/// Full design-token pass over egui defaults. Dark-only.
pub fn apply(ctx: &egui::Context) {
    ctx.options_mut(|o| o.theme_preference = egui::ThemePreference::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 7.0);
        style.spacing.interact_size.y = 32.0;
        style.spacing.combo_width = 200.0;
        style.spacing.scroll.bar_width = 6.0;
        style.spacing.scroll.floating = true;
        for (ts, font) in style.text_styles.iter_mut() {
            match ts {
                egui::TextStyle::Heading => font.size = 18.0,
                egui::TextStyle::Body | egui::TextStyle::Button => font.size = 14.0,
                egui::TextStyle::Small => font.size = 12.0,
                egui::TextStyle::Monospace => font.size = 12.5,
                _ => {}
            }
        }

        let v = &mut style.visuals;
        v.panel_fill = BG;
        v.window_fill = SURFACE;
        v.window_stroke = egui::Stroke::new(1.0, BORDER);
        v.window_corner_radius = egui::CornerRadius::same(12);
        v.menu_corner_radius = egui::CornerRadius::same(8);
        v.window_shadow = egui::epaint::Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: Color32::from_black_alpha(140),
        };
        v.popup_shadow = v.window_shadow;
        v.faint_bg_color = SURFACE;
        v.extreme_bg_color = SURFACE_2; // text inputs
        v.code_bg_color = SURFACE_2;

        let r = egui::CornerRadius::same(8);
        for w in [
            &mut v.widgets.noninteractive,
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            w.corner_radius = r;
            w.expansion = 0.0;
        }
        v.widgets.noninteractive.bg_fill = SURFACE;
        v.widgets.noninteractive.weak_bg_fill = SURFACE;
        v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, BORDER);
        v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, TEXT_2);
        v.widgets.inactive.bg_fill = SURFACE_2;
        v.widgets.inactive.weak_bg_fill = SURFACE_2;
        v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, BORDER);
        v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, FG);
        v.widgets.hovered.bg_fill = SURFACE_3;
        v.widgets.hovered.weak_bg_fill = SURFACE_3;
        v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, RING);
        v.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, FG);
        v.widgets.active.bg_fill = SURFACE_3;
        v.widgets.active.weak_bg_fill = SURFACE_3;
        v.widgets.active.bg_stroke = egui::Stroke::new(1.0, ACCENT);
        v.widgets.active.fg_stroke = egui::Stroke::new(1.0, FG);
        v.widgets.open.bg_fill = SURFACE_2;
        v.widgets.open.weak_bg_fill = SURFACE_2;
        v.widgets.open.bg_stroke = egui::Stroke::new(1.0, RING);
        v.widgets.open.fg_stroke = egui::Stroke::new(1.0, FG);

        v.selection.bg_fill = tint_strong(ACCENT);
        v.selection.stroke = egui::Stroke::new(1.0, ACCENT);
        v.text_cursor.stroke = egui::Stroke::new(2.0, ACCENT);
        v.hyperlink_color = ACCENT;
        v.error_fg_color = RED;
        v.warn_fg_color = AMBER;
        v.override_text_color = None;
    });
}

// ------------------------------------------------------------- components

/// Card: surface fill, hairline ring, radius 12, 20px inset.
pub fn card(_ui: &egui::Ui) -> egui::Frame {
    egui::Frame::default()
        .fill(SURFACE)
        .stroke(egui::Stroke::new(1.0, BORDER))
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(20.0)
}

/// Card heading: SemiBold 15 title over a muted one-line description.
pub fn card_header(ui: &mut egui::Ui, title: &str, desc: &str) {
    ui.spacing_mut().item_spacing.y = 4.0;
    ui.label(egui::RichText::new(title).font(semibold(15.0)).color(FG));
    if !desc.is_empty() {
        ui.label(egui::RichText::new(desc).size(13.0).color(MUTED));
    }
    ui.spacing_mut().item_spacing.y = 8.0;
}

/// Page heading: SemiBold 22 title with an optional muted description.
pub fn page_header(ui: &mut egui::Ui, title: &str, desc: &str) {
    ui.label(egui::RichText::new(title).font(semibold(22.0)).color(FG));
    if !desc.is_empty() {
        ui.add_space(-2.0);
        ui.label(egui::RichText::new(desc).size(13.5).color(TEXT_2));
    }
}

/// Display heading: SemiBold, with the trailing clause in orange ("Three
/// *permissions.*"). Painted at its measured size so it centres cleanly.
pub fn display(ui: &mut egui::Ui, plain: &str, accent: &str, size: f32) {
    let a = ui.fonts(|f| f.layout_no_wrap(plain.to_string(), semibold(size), FG));
    let b = ui.fonts(|f| f.layout_no_wrap(accent.to_string(), semibold(size), ACCENT));
    let total = egui::vec2(a.size().x + b.size().x, a.size().y.max(b.size().y));
    let (rect, _) = ui.allocate_exact_size(total, egui::Sense::hover());
    let p = ui.painter();
    p.galley(rect.min, a.clone(), FG);
    p.galley(egui::pos2(rect.min.x + a.size().x, rect.min.y), b, ACCENT);
}

/// Small section label above a group of cards: medium 12, muted, sentence
/// case. (Uppercase stays reserved for mono machine readouts.)
pub fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(medium(12.5)).color(MUTED));
}

/// Mono uppercase micro-text (timestamps, readouts).
pub fn mono_upper(text: &str, size: f32, color: Color32) -> egui::RichText {
    egui::RichText::new(text.to_uppercase())
        .font(FontId::monospace(size))
        .color(color)
}

/// Keyboard key, shadcn `<Kbd>`: raised surface, hairline ring, a darker
/// bottom edge so it reads as a physical key.
pub fn kbd(ui: &mut egui::Ui, label: &str) -> egui::Response {
    let galley = ui.fonts(|f| f.layout_no_wrap(label.to_string(), mono_medium(12.0), FG));
    let pad = egui::vec2(8.0, 4.0);
    let size = egui::vec2(
        (galley.size().x + pad.x * 2.0).max(26.0),
        galley.size().y + pad.y * 2.0,
    );
    let (rect, resp) = ui.allocate_exact_size(size + egui::vec2(0.0, 2.0), egui::Sense::hover());
    let key = egui::Rect::from_min_size(rect.min, size);
    let p = ui.painter();
    p.rect_filled(
        key.translate(egui::vec2(0.0, 2.0)),
        6.0,
        Color32::from_rgb(4, 4, 4),
    );
    p.rect_filled(key, 6.0, SURFACE_3);
    p.rect_stroke(
        key,
        6.0,
        egui::Stroke::new(1.0, RING),
        egui::StrokeKind::Inside,
    );
    p.galley(
        egui::pos2(key.center().x - galley.size().x / 2.0, key.min.y + pad.y),
        galley,
        FG,
    );
    resp
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // the full kit, not only what today's screens use
pub enum Tone {
    Neutral,
    Accent,
    Danger,
    Warn,
}

impl Tone {
    fn color(self) -> Color32 {
        match self {
            Tone::Neutral => TEXT_2,
            Tone::Accent => ACCENT,
            Tone::Danger => RED,
            Tone::Warn => AMBER,
        }
    }
}

/// Badge, shadcn `<Badge>`: pill, tinted fill, 11.5 medium text.
pub fn badge(ui: &mut egui::Ui, text: &str, tone: Tone) -> egui::Response {
    let c = tone.color();
    let galley = ui.fonts(|f| f.layout_no_wrap(text.to_string(), medium(11.5), c));
    let pad = egui::vec2(8.0, 3.0);
    let (rect, resp) = ui.allocate_exact_size(galley.size() + pad * 2.0, egui::Sense::hover());
    let p = ui.painter();
    let (fill, ring) = if tone == Tone::Neutral {
        (SURFACE_2, BORDER)
    } else {
        (tint(c), tint_strong(c))
    };
    p.rect_filled(rect, rect.height() / 2.0, fill);
    p.rect_stroke(
        rect,
        rect.height() / 2.0,
        egui::Stroke::new(1.0, ring),
        egui::StrokeKind::Inside,
    );
    p.galley(rect.min + pad, galley, c);
    resp
}

/// Status LED: small filled dot with a soft halo. `pulse` animates opacity
/// on a 2s cycle (the caller must keep repainting).
pub fn led(ui: &mut egui::Ui, color: Color32, pulse: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    let a = if pulse {
        let t = ui.input(|i| i.time);
        let phase = (t * std::f64::consts::TAU / 2.0).cos() as f32; // 2s cycle
        0.4 + 0.6 * (0.5 + 0.5 * phase)
    } else {
        1.0
    };
    let p = ui.painter();
    p.circle_filled(rect.center(), 6.5, color.linear_multiply(0.18 * a));
    p.circle_filled(rect.center(), 3.5, color.linear_multiply(a));
}

/// Switch, shadcn `<Switch>`: orange track when on, white thumb always.
pub fn toggle(ui: &mut egui::Ui, on: &mut bool) -> egui::Response {
    let size = egui::vec2(36.0, 20.0);
    let (rect, mut resp) = ui.allocate_exact_size(size, egui::Sense::click());
    if resp.clicked() {
        *on = !*on;
        resp.mark_changed();
    }
    let t = ui.ctx().animate_bool_responsive(resp.id, *on);
    let mix = |a: Color32, b: Color32| {
        let (a, b) = (egui::Rgba::from(a), egui::Rgba::from(b));
        Color32::from(a * (1.0 - t) + b * t)
    };
    let p = ui.painter();
    let track = if resp.hovered() && !*on {
        RING
    } else {
        SURFACE_3
    };
    p.rect_filled(rect, 10.0, mix(track, ACCENT));
    let x = egui::lerp((rect.left() + 10.0)..=(rect.right() - 10.0), t);
    p.circle_filled(egui::pos2(x, rect.center().y), 8.0, FG);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // the full kit, not only what today's screens use
pub enum Variant {
    /// Orange fill: the one high-emphasis action per screen.
    Primary,
    /// Raised neutral fill.
    Secondary,
    /// Transparent with a hairline ring.
    Outline,
    /// No fill, no ring; fills on hover.
    Ghost,
    /// Red text, red fill on hover: irreversible actions.
    Destructive,
}

/// Button, shadcn `<Button>`, with an optional leading phosphor icon. Height
/// 32 (`sm` 28); hover states are part of the variant, not the call site.
pub fn button_with(
    ui: &mut egui::Ui,
    variant: Variant,
    icon: Option<&str>,
    text: &str,
    small: bool,
) -> egui::Response {
    let h = if small { 28.0 } else { 34.0 };
    let font = medium(if small { 12.5 } else { 13.5 });
    let label = match icon {
        Some(i) if text.is_empty() => i.to_string(),
        Some(i) => format!("{i}  {text}"),
        None => text.to_string(),
    };
    let galley = ui.fonts(|f| f.layout_no_wrap(label, font, FG));
    let pad_x = if text.is_empty() {
        (h - galley.size().x) / 2.0
    } else if small {
        10.0
    } else {
        14.0
    };
    let size = egui::vec2(galley.size().x + pad_x * 2.0, h);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());
    let hov = resp.hovered();
    let down = resp.is_pointer_button_down_on();
    let (fill, ring, ink) = match variant {
        Variant::Primary => (
            if hov { ACCENT_HOVER } else { ACCENT },
            Color32::TRANSPARENT,
            ON_ACCENT,
        ),
        Variant::Secondary => (if hov { SURFACE_3 } else { SURFACE_2 }, BORDER, FG),
        Variant::Outline => (
            if hov { SURFACE_2 } else { Color32::TRANSPARENT },
            if hov { RING } else { BORDER },
            if hov { FG } else { TEXT_2 },
        ),
        Variant::Ghost => (
            if hov { SURFACE_2 } else { Color32::TRANSPARENT },
            Color32::TRANSPARENT,
            if hov { FG } else { TEXT_2 },
        ),
        Variant::Destructive => (
            if hov { tint_strong(RED) } else { tint(RED) },
            tint_strong(RED),
            RED,
        ),
    };
    let rect = if down { rect.shrink(0.5) } else { rect };
    let p = ui.painter();
    p.rect_filled(rect, 8.0, fill);
    if ring != Color32::TRANSPARENT {
        p.rect_stroke(
            rect,
            8.0,
            egui::Stroke::new(1.0, ring),
            egui::StrokeKind::Inside,
        );
    }
    p.galley(rect.center() - galley.size() / 2.0, galley, ink);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

pub fn button(ui: &mut egui::Ui, variant: Variant, text: &str) -> egui::Response {
    button_with(ui, variant, None, text, false)
}

/// The one high-emphasis action per screen.
pub fn primary_button(ui: &mut egui::Ui, text: impl Into<String>) -> egui::Response {
    button(ui, Variant::Primary, &text.into())
}

/// Sidebar navigation item: icon + label, full width, 34px tall. Selected =
/// raised fill, orange icon and a 2px orange rail on the left edge.
pub fn nav_item(ui: &mut egui::Ui, icon: &str, label: &str, selected: bool) -> egui::Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 34.0), egui::Sense::click());
    let hov = resp.hovered();
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 8.0, SURFACE_2);
        p.rect_stroke(
            rect,
            8.0,
            egui::Stroke::new(1.0, BORDER),
            egui::StrokeKind::Inside,
        );
        p.rect_filled(
            egui::Rect::from_min_size(
                egui::pos2(rect.left(), rect.top() + 9.0),
                egui::vec2(2.5, rect.height() - 18.0),
            ),
            1.5,
            ACCENT,
        );
    } else if hov {
        p.rect_filled(rect, 8.0, SURFACE);
    }
    let cy = rect.center().y;
    p.text(
        egui::pos2(rect.left() + 14.0, cy),
        egui::Align2::LEFT_CENTER,
        icon,
        FontId::proportional(16.0),
        if selected {
            ACCENT
        } else if hov {
            FG
        } else {
            MUTED
        },
    );
    p.text(
        egui::pos2(rect.left() + 40.0, cy),
        egui::Align2::LEFT_CENTER,
        label,
        medium(13.5),
        if selected || hov { FG } else { TEXT_2 },
    );
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Thin determinate progress bar: orange fill on a surface track.
pub fn progress(ui: &mut egui::Ui, frac: f32) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 6.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 3.0, SURFACE_3);
    let mut fill = rect;
    fill.set_width(rect.width() * frac.clamp(0.0, 1.0));
    p.rect_filled(fill, 3.0, ACCENT);
}

/// 1px horizontal hairline across the available width.
pub fn hairline(ui: &mut egui::Ui) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 1.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, BORDER);
}

/// The app icon (assets/icon-128.png), drawn at `size` points. Decoded once
/// per window and cached in egui's memory.
pub fn logo(ui: &mut egui::Ui, size: f32) -> egui::Response {
    let tex = logo_texture(ui.ctx());
    ui.add(egui::Image::new((tex.id(), egui::vec2(size, size))))
}

fn logo_texture(ctx: &egui::Context) -> egui::TextureHandle {
    let id = egui::Id::new("wc-logo-texture");
    if let Some(t) = ctx.data(|d| d.get_temp::<egui::TextureHandle>(id)) {
        return t;
    }
    let img = image::load_from_memory(include_bytes!("../../../assets/icon-128.png"))
        .expect("bundled logo decodes")
        .to_rgba8();
    let (w, h) = img.dimensions();
    let color = egui::ColorImage::from_rgba_unmultiplied([w as usize, h as usize], img.as_raw());
    let t = ctx.load_texture("wc-logo", color, egui::TextureOptions::LINEAR);
    ctx.data_mut(|d| d.insert_temp(id, t.clone()));
    t
}
