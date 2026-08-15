//! Shared look & feel for all app windows — "tactile engineer dark".
//!
//! Every value comes from docs/DESIGN.md (Part B). Dark only: there is no
//! light theme and no theme picker. The palette is a warm near-black cousin
//! of the website's cream, and it shares the site's one accent — mint —
//! so the app and the landing page read as the same product.
//!
//! Screens never hand-pick colors; they use the tokens and helpers below.

use eframe::egui::{self, Color32, FontFamily, FontId};

// ---------------------------------------------------------------- palette
// Warm neutrals. Slightly green-shifted rather than pure zinc, so the app
// feels related to the site's paper instead of like a different product.

pub const BG: Color32 = Color32::from_rgb(11, 13, 12); // window
pub const SURFACE: Color32 = Color32::from_rgb(20, 24, 23); // cards
pub const SURFACE_2: Color32 = Color32::from_rgb(28, 33, 32); // raised controls
pub const SURFACE_3: Color32 = Color32::from_rgb(38, 44, 42); // hover/active
pub const FG: Color32 = Color32::from_rgb(233, 239, 236); // primary text
pub const TEXT_2: Color32 = Color32::from_rgb(154, 165, 160); // secondary text
pub const MUTED: Color32 = Color32::from_rgb(110, 120, 115); // labels, metadata
/// 1px hairline — white at ~8% over BG.
pub const BORDER: Color32 = Color32::from_rgb(30, 35, 34);
/// Focus/selected ring — white at ~20%.
pub const RING: Color32 = Color32::from_rgb(51, 59, 57);

// accent — the website's mint, and the deep green it sits on there
pub const MINT: Color32 = Color32::from_rgb(93, 232, 205);
/// Text on a mint fill.
pub const ON_MINT: Color32 = Color32::from_rgb(6, 52, 44);
// signal colors — state only, never decoration
pub const RED: Color32 = Color32::from_rgb(239, 95, 82); // recording
pub const AMBER: Color32 = Color32::from_rgb(240, 169, 76); // processing / hotkey

/// `color` at ~9% alpha — chip fills behind signal-colored text.
pub fn tint(color: Color32) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 22)
}

/// `color` at ~18% alpha — rings and edges around a tinted plate.
pub fn tint_strong(color: Color32) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 46)
}

// ------------------------------------------------------------------ fonts

/// Geist (sans) + Geist Mono + Newsreader (serif display), all embedded;
/// egui-phosphor appended for icons. Families: `Proportional` → Geist,
/// `Monospace` → Geist Mono, plus named "GeistMedium" / "GeistSemiBold" /
/// "GeistMonoMedium" for emphasis (egui's `strong()` only recolors — weight
/// needs a family switch) and "Serif" / "SerifItalic" for display type.
///
/// Newsreader is the same face the website sets its headlines in; it is what
/// makes the two surfaces look like one product.
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let data: [(&str, &[u8]); 7] = [
        ("geist", include_bytes!("../assets/fonts/Geist-Regular.ttf")),
        ("geist-medium", include_bytes!("../assets/fonts/Geist-Medium.ttf")),
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
        (
            "newsreader",
            include_bytes!("../assets/fonts/Newsreader-Regular.ttf"),
        ),
        (
            "newsreader-italic",
            include_bytes!("../assets/fonts/Newsreader-Italic.ttf"),
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
        ("Serif", "newsreader", &prop),
        ("SerifItalic", "newsreader-italic", &prop),
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

/// Newsreader — display type only (window titles, step titles, empty states).
pub fn serif(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("Serif".into()))
}

/// Newsreader italic — the emphasis half of a display line, as on the site.
pub fn serif_italic(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("SerifItalic".into()))
}

// ------------------------------------------------------------------ style

/// Full design-token pass over egui defaults. Dark-only.
pub fn apply(ctx: &egui::Context) {
    ctx.options_mut(|o| o.theme_preference = egui::ThemePreference::Dark);
    ctx.style_mut_of(egui::Theme::Dark, |style| {
        style.spacing.item_spacing = egui::vec2(8.0, 8.0);
        style.spacing.button_padding = egui::vec2(12.0, 7.0);
        style.spacing.interact_size.y = 28.0;
        style.spacing.scroll.bar_width = 8.0;
        for (ts, font) in style.text_styles.iter_mut() {
            match ts {
                egui::TextStyle::Heading => font.size = 17.0,
                egui::TextStyle::Body | egui::TextStyle::Button => font.size = 14.0,
                egui::TextStyle::Small => font.size = 11.5,
                egui::TextStyle::Monospace => font.size = 12.0,
                _ => {}
            }
        }

        let v = &mut style.visuals;
        v.panel_fill = BG;
        v.window_fill = SURFACE;
        v.window_stroke = egui::Stroke::new(1.0, BORDER);
        v.window_corner_radius = egui::CornerRadius::same(14);
        v.menu_corner_radius = egui::CornerRadius::same(10);
        v.faint_bg_color = SURFACE;
        v.extreme_bg_color = Color32::from_rgb(15, 18, 17); // inputs

        let r = egui::CornerRadius::same(8);
        for w in [
            &mut v.widgets.noninteractive,
            &mut v.widgets.inactive,
            &mut v.widgets.hovered,
            &mut v.widgets.active,
            &mut v.widgets.open,
        ] {
            w.corner_radius = r;
        }
        v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, BORDER);
        v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, FG);
        v.widgets.inactive.bg_fill = SURFACE_2;
        v.widgets.inactive.weak_bg_fill = SURFACE_2;
        v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, BORDER);
        v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, TEXT_2);
        v.widgets.hovered.bg_fill = SURFACE_3;
        v.widgets.hovered.weak_bg_fill = SURFACE_3;
        v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, RING);
        v.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, FG);
        v.widgets.active.bg_fill = SURFACE_3;
        v.widgets.active.weak_bg_fill = SURFACE_3;
        v.widgets.active.bg_stroke = egui::Stroke::new(1.0, RING);
        v.widgets.active.fg_stroke = egui::Stroke::new(1.0, FG);
        v.widgets.open.bg_fill = SURFACE_2;
        v.widgets.open.weak_bg_fill = SURFACE_2;
        v.widgets.open.bg_stroke = egui::Stroke::new(1.0, RING);
        v.widgets.open.fg_stroke = egui::Stroke::new(1.0, FG);

        v.selection.bg_fill = tint_strong(MINT);
        v.selection.stroke = egui::Stroke::new(1.0, MINT);
        v.hyperlink_color = MINT;
        v.error_fg_color = RED;
        v.warn_fg_color = AMBER;
        v.override_text_color = None;
    });
}

// ------------------------------------------------------------- components

/// Card container: surface fill, hairline ring, radius 12, 18px inset.
pub fn card(_ui: &egui::Ui) -> egui::Frame {
    egui::Frame::default()
        .fill(SURFACE)
        .stroke(egui::Stroke::new(1.0, BORDER))
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(18.0)
}

/// Display heading in Newsreader, with the trailing clause in italic — the
/// same two-tone headline the website uses ("You talk. *It types.*").
///
/// Painted at its exact measured size rather than laid out as two labels, so
/// it stays centered inside a `vertical_centered` and the roman and italic
/// halves sit tight against each other.
pub fn display(ui: &mut egui::Ui, roman: &str, italic: &str, size: f32) {
    let roman_g = ui.fonts(|f| f.layout_no_wrap(roman.to_string(), serif(size), FG));
    let italic_g =
        ui.fonts(|f| f.layout_no_wrap(italic.to_string(), serif_italic(size), FG));
    let total = egui::vec2(
        roman_g.size().x + italic_g.size().x,
        roman_g.size().y.max(italic_g.size().y),
    );
    let (rect, _) = ui.allocate_exact_size(total, egui::Sense::hover());
    let p = ui.painter();
    p.galley(rect.min, roman_g.clone(), FG);
    p.galley(
        egui::pos2(rect.min.x + roman_g.size().x, rect.min.y),
        italic_g,
        FG,
    );
}

/// Small mono uppercase section label ("ENGINE PARAMETERS").
pub fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(
        egui::RichText::new(text.to_uppercase())
            .font(mono_medium(11.0))
            .color(MUTED),
    );
}

/// Mono uppercase micro-text (timestamps, readouts).
pub fn mono_upper(text: &str, size: f32, color: Color32) -> egui::RichText {
    egui::RichText::new(text.to_uppercase())
        .font(FontId::monospace(size))
        .color(color)
}

/// Hotkey chip: amber mono uppercase on an amber tint, radius 5, with the
/// bottom edge that makes it read as a physical key.
pub fn key_chip(ui: &mut egui::Ui, label: &str) {
    egui::Frame::default()
        .fill(tint(AMBER))
        .stroke(egui::Stroke::new(1.0, tint_strong(AMBER)))
        .corner_radius(egui::CornerRadius::same(5))
        .inner_margin(egui::Margin::symmetric(9, 5))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(label.to_uppercase())
                    .font(mono_medium(11.0))
                    .color(AMBER),
            );
        });
}

/// Status LED: small filled dot with a soft halo. `pulse` animates opacity
/// on a 2s cycle (caller must keep repainting, e.g. the overlay).
pub fn led(ui: &mut egui::Ui, color: Color32, pulse: bool) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
    let a = if pulse {
        let t = ui.input(|i| i.time);
        let phase = (t * std::f64::consts::TAU / 2.0).cos() as f32; // 2s cycle
        0.4 + 0.6 * (0.5 + 0.5 * phase)
    } else {
        1.0
    };
    let c = color.linear_multiply(a);
    let p = ui.painter();
    p.circle_filled(rect.center(), 7.0, color.linear_multiply(0.16 * a));
    p.circle_filled(rect.center(), 3.5, c);
}

/// Hardware-style toggle switch — mint when on.
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
    p.rect_filled(rect, 10.0, mix(SURFACE_3, MINT));
    p.rect_stroke(
        rect,
        10.0,
        egui::Stroke::new(1.0, mix(RING, MINT)),
        egui::StrokeKind::Inside,
    );
    let knob = mix(FG, ON_MINT);
    let x = egui::lerp((rect.left() + 10.0)..=(rect.right() - 10.0), t);
    p.circle_filled(egui::pos2(x, rect.center().y), 7.0, knob);
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// The one high-emphasis action per screen: mint fill, deep-green text —
/// the website's button, in the dark.
pub fn primary_button(ui: &mut egui::Ui, text: impl Into<String>) -> egui::Response {
    ui.add(
        egui::Button::new(
            egui::RichText::new(text.into())
                .font(medium(13.5))
                .color(ON_MINT),
        )
        .fill(MINT)
        .stroke(egui::Stroke::NONE)
        .corner_radius(egui::CornerRadius::same(8)),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Quiet secondary action: no fill, hairline ring.
pub fn ghost_button(ui: &mut egui::Ui, text: impl Into<String>) -> egui::Response {
    ui.add(
        egui::Button::new(
            egui::RichText::new(text.into())
                .font(medium(13.0))
                .color(TEXT_2),
        )
        .fill(Color32::TRANSPARENT)
        .stroke(egui::Stroke::new(1.0, RING))
        .corner_radius(egui::CornerRadius::same(8)),
    )
    .on_hover_cursor(egui::CursorIcon::PointingHand)
}
