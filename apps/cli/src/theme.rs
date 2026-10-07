//! Shared look & feel for all app windows: "charcoal + yellow".
//!
//! Every value comes from docs/DESIGN.md (Part B). Two palettes, [`DARK`] and
//! [`LIGHT`], chosen per frame from the OS theme (`WC_THEME=light|dark`
//! overrides it for captures); there is no theme picker. Screens read colours
//! only through the token functions (`theme::fg()`, `theme::accent_ink()`...)
//! and a small kit of components (buttons in five variants, badge, kbd,
//! switch, nav item), so nothing hand-picks a colour or restyles a widget
//! inline. Yellow is a fill (`accent`) everywhere; as text, icon or thin
//! stroke it is `accent_ink`, which is charcoal in light mode.

use eframe::egui::{self, Color32, FontFamily, FontId};
use std::sync::atomic::{AtomicBool, Ordering};

// ---------------------------------------------------------------- palette
// Charcoal + slate + yellow + light grey, nothing else (plus RED for errors).
// Every token resolves per theme: `theme::fg()` returns the dark or light
// value depending on the theme chosen at the start of the frame by `apply`.
// Depth comes from fill + hairline, never from a shadow.

/// One full set of token values.
pub struct Palette {
    pub bg: Color32,
    pub sheet: Color32,
    pub panel: Color32,
    pub sidebar: Color32,
    pub surface: Color32,
    pub surface_2: Color32,
    pub surface_3: Color32,
    pub fg: Color32,
    pub text_2: Color32,
    pub muted: Color32,
    pub border: Color32,
    pub ring: Color32,
    pub accent: Color32,
    pub accent_hover: Color32,
    pub on_accent: Color32,
    pub accent_ink: Color32,
    pub red: Color32,
    pub amber: Color32,
    pub scrim: Color32,
}

const YELLOW: Color32 = Color32::from_rgb(0xff, 0xd3, 0x69);
const CHARCOAL: Color32 = Color32::from_rgb(0x22, 0x28, 0x31);
const ERROR_RED: Color32 = Color32::from_rgb(0xef, 0x44, 0x44);

const fn rgb(hex: u32) -> Color32 {
    Color32::from_rgb((hex >> 16) as u8, (hex >> 8) as u8, hex as u8)
}

pub const DARK: Palette = Palette {
    bg: CHARCOAL,
    sheet: rgb(0x2e333c),
    panel: rgb(0x393e46),
    sidebar: rgb(0x282e36),
    surface: rgb(0x30353e),
    surface_2: rgb(0x464a52),
    surface_3: rgb(0x54585f),
    fg: rgb(0xeeeeee),
    text_2: rgb(0xb8b9bc),
    muted: rgb(0xa6a8ab),
    border: rgb(0x494e55),
    ring: rgb(0x686c72),
    accent: YELLOW,
    accent_hover: rgb(0xf9dc98),
    on_accent: CHARCOAL,
    accent_ink: YELLOW,
    red: ERROR_RED,
    amber: YELLOW,
    scrim: Color32::from_rgba_premultiplied(0x1d, 0x22, 0x29, 215),
};

pub const LIGHT: Palette = Palette {
    bg: rgb(0xe1e2e2),
    sheet: rgb(0xeeeeee),
    panel: rgb(0xe8e8e8),
    sidebar: rgb(0xe7e7e7),
    surface: rgb(0xe5e5e6),
    surface_2: rgb(0xe0e0e1),
    surface_3: rgb(0xd5d5d6),
    fg: CHARCOAL,
    text_2: rgb(0x393e46),
    muted: rgb(0x61656b),
    border: rgb(0xd3d4d5),
    ring: rgb(0xb1b3b5),
    accent: YELLOW,
    accent_hover: rgb(0xedc565),
    on_accent: CHARCOAL,
    accent_ink: CHARCOAL,
    red: ERROR_RED,
    amber: CHARCOAL,
    scrim: Color32::from_rgba_premultiplied(0x10, 0x13, 0x17, 120),
};

static IS_DARK: AtomicBool = AtomicBool::new(true);

/// Select the palette every token accessor resolves against.
pub fn set_dark(dark: bool) {
    IS_DARK.store(dark, Ordering::Relaxed);
}

pub fn is_dark() -> bool {
    IS_DARK.load(Ordering::Relaxed)
}

/// The palette for the current theme.
pub fn pal() -> &'static Palette {
    if is_dark() {
        &DARK
    } else {
        &LIGHT
    }
}

macro_rules! token {
    ($($(#[$m:meta])* $name:ident),* $(,)?) => {
        $($(#[$m])* pub fn $name() -> Color32 { pal().$name })*
    };
}

token! {
    /// Window ground.
    bg,
    /// Rounded content sheet the pages sit on, a step above the window.
    sheet,
    /// Cards inside the sheet.
    panel,
    /// Nav rail.
    sidebar,
    /// Popups, nav hover.
    surface,
    /// Inputs, secondary buttons.
    surface_2,
    /// Hover / active fills, key caps, switch off.
    surface_3,
    /// Primary text.
    fg,
    /// Secondary text.
    text_2,
    /// Labels, metadata.
    muted,
    /// 1px hairline.
    border,
    /// Hover / focus ring.
    ring,
    /// The one accent, yellow, as a FILL: primary button, switch on,
    /// progress, bars, heatmap, badges.
    accent,
    /// Hovered primary button.
    accent_hover,
    /// Text and glyphs on a yellow fill.
    on_accent,
    /// The accent as TEXT, icon, thin stroke, caret, rail, underline: yellow
    /// on dark, charcoal on light (yellow on light grey is 1.2:1).
    accent_ink,
    /// Errors and destructive actions only.
    red,
    /// Advisories that still work; same ink as the accent.
    amber,
    /// Scrim behind modals.
    scrim,
}

/// Darker bottom edge of a key cap.
pub fn key_edge() -> Color32 {
    if is_dark() {
        mix(bg(), Color32::BLACK, 0.4)
    } else {
        ring()
    }
}

/// Hairline edge for a yellow fill that sits directly on a light surface
/// (switch track, progress, bars); transparent in dark mode.
pub fn fill_edge() -> Color32 {
    if is_dark() {
        Color32::TRANSPARENT
    } else {
        mix(YELLOW, CHARCOAL, 0.35)
    }
}

/// `color` at ~9% alpha: badge fills behind signal-coloured text.
pub fn tint(color: Color32) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 22)
}

/// `color` at ~18% alpha: the ring around a tinted plate.
pub fn tint_strong(color: Color32) -> Color32 {
    Color32::from_rgba_unmultiplied(color.r(), color.g(), color.b(), 46)
}

/// `a` blended toward `b` by `t` (0 to 1), per channel.
pub fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgb(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()))
}

/// Heatmap ramp: level 0 is an empty cell (`SURFACE_3`); levels 1 to 4 mix
/// `ACCENT` toward the card (`PANEL`) the cells sit on, strongest last.
pub fn heat(level: u8) -> Color32 {
    if !is_dark() {
        // Light: ramp from the empty cell toward yellow; the top level is
        // darkened a touch so it still reads against the pale card.
        return match level {
            0 => surface_3(),
            1 => mix(surface_3(), YELLOW, 0.40),
            2 => mix(surface_3(), YELLOW, 0.70),
            3 => YELLOW,
            _ => mix(YELLOW, CHARCOAL, 0.25),
        };
    }
    match level {
        0 => surface_3(),
        1 => mix(panel(), accent(), 0.30),
        2 => mix(panel(), accent(), 0.52),
        3 => mix(panel(), accent(), 0.76),
        _ => accent(),
    }
}

// ------------------------------------------------------------------ fonts

/// Geist (sans) + Geist Mono, embedded; egui-phosphor appended for icons.
/// Families: `Proportional` → Geist, `Monospace` → Geist Mono, plus named
/// "GeistMedium" / "GeistSemiBold" / "GeistMonoMedium" for emphasis (egui's
/// `strong()` only recolors, so weight needs a family switch).
pub fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let data: [(&str, &[u8]); 6] = [
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
        (
            "newsreader",
            include_bytes!("../assets/fonts/Newsreader-Regular.ttf"),
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
    // Newsreader: display headlines only (hero banner, modal titles).
    let mut serif = fonts.families[&FontFamily::Proportional].clone();
    serif.insert(0, "newsreader".to_owned());
    fonts
        .families
        .insert(FontFamily::Name("Newsreader".into()), serif);
    ctx.set_fonts(fonts);
}

/// Newsreader Regular: the serif for display headlines.
pub fn serif(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name("Newsreader".into()))
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

/// Which theme to start in: `WC_THEME=light|dark` forces it (screenshots and
/// debugging), otherwise the OS setting decides.
fn preference() -> egui::ThemePreference {
    match std::env::var("WC_THEME").as_deref() {
        Ok("light") => egui::ThemePreference::Light,
        Ok("dark") => egui::ThemePreference::Dark,
        _ => egui::ThemePreference::System,
    }
}

/// Call at the top of every window's `update`: points the token accessors
/// at the palette of the theme egui resolved for this frame.
pub fn begin_frame(ctx: &egui::Context) {
    set_dark(ctx.theme() == egui::Theme::Dark);
}

/// Full design-token pass over egui defaults, for both themes. Follows the OS
/// light/dark setting; spacing and text styles are identical in both.
pub fn apply(ctx: &egui::Context) {
    ctx.options_mut(|o| o.theme_preference = preference());
    begin_frame(ctx);
    configure(ctx, egui::Theme::Dark, &DARK);
    configure(ctx, egui::Theme::Light, &LIGHT);
}

fn configure(ctx: &egui::Context, theme: egui::Theme, p: &Palette) {
    let dark = theme == egui::Theme::Dark;
    ctx.style_mut_of(theme, |style| {
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
        v.panel_fill = p.bg;
        v.window_fill = p.surface;
        v.window_stroke = egui::Stroke::new(1.0, p.border);
        v.window_corner_radius = egui::CornerRadius::same(12);
        v.menu_corner_radius = egui::CornerRadius::same(8);
        v.window_shadow = egui::epaint::Shadow {
            offset: [0, 8],
            blur: 24,
            spread: 0,
            color: Color32::from_black_alpha(if dark { 140 } else { 45 }),
        };
        v.popup_shadow = v.window_shadow;
        v.faint_bg_color = p.surface;
        v.extreme_bg_color = p.surface_2; // text inputs
        v.code_bg_color = p.surface_2;

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
        v.widgets.noninteractive.bg_fill = p.surface;
        v.widgets.noninteractive.weak_bg_fill = p.surface;
        v.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, p.border);
        v.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, p.text_2);
        v.widgets.inactive.bg_fill = p.surface_2;
        v.widgets.inactive.weak_bg_fill = p.surface_2;
        v.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, p.border);
        v.widgets.inactive.fg_stroke = egui::Stroke::new(1.0, p.fg);
        v.widgets.hovered.bg_fill = p.surface_3;
        v.widgets.hovered.weak_bg_fill = p.surface_3;
        v.widgets.hovered.bg_stroke = egui::Stroke::new(1.0, p.ring);
        v.widgets.hovered.fg_stroke = egui::Stroke::new(1.0, p.fg);
        v.widgets.active.bg_fill = p.surface_3;
        v.widgets.active.weak_bg_fill = p.surface_3;
        v.widgets.active.bg_stroke = egui::Stroke::new(1.0, p.accent_ink);
        v.widgets.active.fg_stroke = egui::Stroke::new(1.0, p.fg);
        v.widgets.open.bg_fill = p.surface_2;
        v.widgets.open.weak_bg_fill = p.surface_2;
        v.widgets.open.bg_stroke = egui::Stroke::new(1.0, p.ring);
        v.widgets.open.fg_stroke = egui::Stroke::new(1.0, p.fg);

        v.selection.bg_fill = if dark { tint_strong(p.accent) } else { p.accent.gamma_multiply(0.6) };
        v.selection.stroke = egui::Stroke::new(1.0, p.accent_ink);
        v.text_cursor.stroke = egui::Stroke::new(2.0, p.accent_ink);
        v.hyperlink_color = p.accent_ink;
        v.error_fg_color = p.red;
        v.warn_fg_color = p.amber;
        v.override_text_color = None;
    });
}

// ------------------------------------------------------------- components

/// Card: panel fill (a step above the sheet), hairline ring, radius 12, 20px inset.
pub fn card(_ui: &egui::Ui) -> egui::Frame {
    egui::Frame::default()
        .fill(panel())
        .stroke(egui::Stroke::new(1.0, border()))
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(20.0)
}

/// Card heading: SemiBold 15 title over a muted one-line description.
pub fn card_header(ui: &mut egui::Ui, title: &str, desc: &str) {
    ui.spacing_mut().item_spacing.y = 4.0;
    ui.label(egui::RichText::new(title).font(semibold(15.0)).color(fg()));
    if !desc.is_empty() {
        ui.label(egui::RichText::new(desc).size(13.0).color(muted()));
    }
    ui.spacing_mut().item_spacing.y = 8.0;
}

/// Page heading: SemiBold 28 title with an optional muted description.
pub fn page_header(ui: &mut egui::Ui, title: &str, desc: &str) {
    ui.label(egui::RichText::new(title).font(semibold(28.0)).color(fg()));
    if !desc.is_empty() {
        ui.add_space(2.0);
        ui.label(egui::RichText::new(desc).size(14.0).color(text_2()));
    }
}

/// Page heading with an optional badge beside the title and controls
/// right-aligned on the same line. `controls` runs right-to-left (the first
/// widget added is the rightmost).
pub fn page_header_with(
    ui: &mut egui::Ui,
    title: &str,
    badge_text: Option<&str>,
    controls: impl FnOnce(&mut egui::Ui),
) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        ui.label(egui::RichText::new(title).font(semibold(28.0)).color(fg()));
        if let Some(b) = badge_text {
            badge(ui, b, Tone::Neutral);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), controls);
    });
}

/// Reusable banner: radius 16, dark gradient with a warm yellow glow, serif
/// headline, TEXT_2 body, and `actions` (buttons) underneath. The gradient is
/// a vertex-coloured mesh built only from theme tokens; a rounded-rect SDF
/// fades the mesh at its edge so the corners stay round.
pub fn hero<R>(
    ui: &mut egui::Ui,
    headline: &str,
    body: &str,
    actions: impl FnOnce(&mut egui::Ui) -> R,
) -> R {
    let bg_idx = ui.painter().add(egui::Shape::Noop);
    let out = egui::Frame::default()
        .inner_margin(egui::Margin::symmetric(36, 32))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            ui.label(egui::RichText::new(headline).font(serif(36.0)).color(fg()));
            ui.add_space(10.0);
            ui.label(egui::RichText::new(body).size(14.5).color(text_2()));
            ui.add_space(20.0);
            actions(ui)
        });
    let rect = out.response.rect;
    let radius = 16.0_f32;
    let mut mesh = egui::Mesh::default();
    let step = 6.0;
    let nx = (rect.width() / step).ceil().max(1.0) as usize;
    let ny = (rect.height() / step).ceil().max(1.0) as usize;
    // Glow centre: upper right, so the headline side stays dark.
    let glow = egui::pos2(
        rect.right() - rect.width() * 0.2,
        rect.top() + rect.height() * 0.15,
    );
    let reach = rect.width() * 0.55;
    let half = rect.size() / 2.0;
    for j in 0..=ny {
        for i in 0..=nx {
            let pos = egui::pos2(
                rect.left() + rect.width() * i as f32 / nx as f32,
                rect.top() + rect.height() * j as f32 / ny as f32,
            );
            let d = ((pos - glow).length() / reach).clamp(0.0, 1.0);
            let a = ((1.0 - d).powi(2) * 0.55).clamp(0.0, 1.0);
            let mix = |b: u8, g: u8| (b as f32 * (1.0 - a) + g as f32 * a) as u8;
            let col = Color32::from_rgb(
                mix(panel().r(), accent().r()),
                mix(panel().g(), accent().g()),
                mix(panel().b(), accent().b()),
            );
            // Rounded-rect signed distance: negative inside.
            // Only the corners fade: along the straight edges the mesh is
            // coarser than any ramp, so a fade there would show as a band.
            let q = (pos - rect.center()).abs() - (half - egui::vec2(radius, radius));
            let cover = if q.x > 0.0 && q.y > 0.0 {
                ((radius - q.length()) / 3.0 + 0.5).clamp(0.0, 1.0)
            } else {
                1.0
            };
            mesh.colored_vertex(pos, col.gamma_multiply(cover));
        }
    }
    for j in 0..ny {
        for i in 0..nx {
            let a = (j * (nx + 1) + i) as u32;
            let b = a + 1;
            let c = a + (nx + 1) as u32;
            let d = c + 1;
            mesh.add_triangle(a, b, c);
            mesh.add_triangle(b, d, c);
        }
    }
    ui.painter().set(
        bg_idx,
        egui::Shape::Vec(vec![
            egui::Shape::mesh(mesh),
            egui::Shape::rect_stroke(
                rect,
                radius,
                egui::Stroke::new(1.0, border()),
                egui::StrokeKind::Inside,
            ),
        ]),
    );
    out.inner
}

fn row_first_id() -> egui::Id {
    egui::Id::new("wc-row-first")
}

/// A group of settings rows in one card: panel fill, hairline ring, radius
/// 12. Rows ([`row`]) are separated by hairlines inside it.
pub fn group<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.data_mut(|d| d.insert_temp(row_first_id(), true));
    egui::Frame::default()
        .fill(panel())
        .stroke(egui::Stroke::new(1.0, border()))
        .corner_radius(egui::CornerRadius::same(12))
        .inner_margin(egui::Margin::symmetric(24, 0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 0.0;
            add(ui)
        })
        .inner
}

/// One row of a [`group`]: title (medium 15) over a muted description on the
/// left, the control on the right (right-to-left layout), 16px of air above
/// and below, and a hairline above every row but the first.
pub fn row(ui: &mut egui::Ui, title: &str, desc: &str, control: impl FnOnce(&mut egui::Ui)) {
    let first = ui.data_mut(|d| d.get_temp::<bool>(row_first_id()).unwrap_or(false));
    if first {
        ui.data_mut(|d| d.insert_temp(row_first_id(), false));
    } else {
        hairline(ui);
    }
    ui.add_space(16.0);
    let full = ui.available_width();
    let left_w = (full - 220.0).max(full * 0.5);
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.set_max_width(left_w);
            ui.spacing_mut().item_spacing.y = 4.0;
            ui.label(egui::RichText::new(title).font(medium(15.0)).color(fg()));
            if !desc.is_empty() {
                ui.label(egui::RichText::new(desc).size(13.5).color(text_2()));
            }
        });
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), control);
    });
    ui.add_space(16.0);
}

/// Display heading: SemiBold, with the trailing clause in yellow ("Three
/// *permissions.*"). Painted at its measured size so it centres cleanly.
pub fn display(ui: &mut egui::Ui, plain: &str, tail: &str, size: f32) {
    let a = ui.fonts(|f| f.layout_no_wrap(plain.to_string(), semibold(size), fg()));
    let b = ui.fonts(|f| f.layout_no_wrap(tail.to_string(), semibold(size), accent_ink()));
    let total = egui::vec2(a.size().x + b.size().x, a.size().y.max(b.size().y));
    let (rect, _) = ui.allocate_exact_size(total, egui::Sense::hover());
    let p = ui.painter();
    p.galley(rect.min, a.clone(), fg());
    p.galley(egui::pos2(rect.min.x + a.size().x, rect.min.y), b, accent_ink());
}

/// Small section label above a group of cards: medium 12, muted, sentence
/// case. (Uppercase stays reserved for mono machine readouts.)
pub fn section_label(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).font(medium(12.5)).color(muted()));
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
    let galley = ui.fonts(|f| f.layout_no_wrap(label.to_string(), mono_medium(12.0), fg()));
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
        key_edge(),
    );
    p.rect_filled(key, 6.0, surface_3());
    p.rect_stroke(
        key,
        6.0,
        egui::Stroke::new(1.0, ring()),
        egui::StrokeKind::Inside,
    );
    p.galley(
        egui::pos2(key.center().x - galley.size().x / 2.0, key.min.y + pad.y),
        galley,
        fg(),
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
            Tone::Neutral => text_2(),
            Tone::Accent => accent_ink(),
            Tone::Danger => red(),
            Tone::Warn => amber(),
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
        (surface_2(), border())
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

/// Switch, shadcn `<Switch>`: yellow track when on, white thumb always.
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
        ring()
    } else {
        surface_3()
    };
    p.rect_filled(rect, 10.0, mix(track, accent()));
    if t > 0.0 && !is_dark() {
        p.rect_stroke(
            rect,
            10.0,
            egui::Stroke::new(1.0, fill_edge().gamma_multiply(t)),
            egui::StrokeKind::Inside,
        );
    }
    let x = egui::lerp((rect.left() + 10.0)..=(rect.right() - 10.0), t);
    p.circle_filled(egui::pos2(x, rect.center().y), 8.0, fg());
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

#[derive(Clone, Copy, PartialEq, Eq)]
#[allow(dead_code)] // the full kit, not only what today's screens use
pub enum Variant {
    /// Yellow fill: the one high-emphasis action per screen.
    Primary,
    /// Raised neutral fill.
    Secondary,
    /// Transparent with a hairline ring.
    Outline,
    /// No fill, no ring; fills on hover.
    Ghost,
    /// Red text, red fill on hover: irreversible actions.
    Destructive,
    /// Light fill, dark text: the button on a hero banner.
    Light,
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
    // PLACEHOLDER: the glyph colour is the `ink` chosen below, once hover is known.
    let ink0 = Color32::PLACEHOLDER;
    let galley = ui.fonts(|f| f.layout_no_wrap(label, font, ink0));
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
            if hov { accent_hover() } else { accent() },
            fill_edge(),
            on_accent(),
        ),
        Variant::Secondary => (if hov { surface_3() } else { surface_2() }, border(), fg()),
        Variant::Outline => (
            if hov { surface_2() } else { Color32::TRANSPARENT },
            if hov { ring() } else { border() },
            if hov { fg() } else { text_2() },
        ),
        Variant::Ghost => (
            if hov { surface_2() } else { Color32::TRANSPARENT },
            Color32::TRANSPARENT,
            if hov { fg() } else { text_2() },
        ),
        Variant::Destructive => (
            if hov { tint_strong(red()) } else { tint(red()) },
            tint_strong(red()),
            red(),
        ),
        Variant::Light => (
            if hov { fg().gamma_multiply(0.88) } else { fg() },
            Color32::TRANSPARENT,
            bg(),
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

/// Sidebar navigation item: icon + label, full width, 38px tall, radius 8.
/// Selected = raised fill, FG label and yellow icon; hover = soft fill.
pub fn nav_item(ui: &mut egui::Ui, icon: &str, label: &str, selected: bool) -> egui::Response {
    nav_item_with_badge(ui, icon, label, selected, None)
}

/// [`nav_item`] with an optional yellow badge ("New") on the right edge.
pub fn nav_item_with_badge(
    ui: &mut egui::Ui,
    icon: &str,
    label: &str,
    selected: bool,
    badge_text: Option<&str>,
) -> egui::Response {
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 38.0), egui::Sense::click());
    let hov = resp.hovered();
    let p = ui.painter();
    if selected {
        p.rect_filled(rect, 8.0, surface_2());
    } else if hov {
        p.rect_filled(rect, 8.0, surface());
    }
    let cy = rect.center().y;
    p.text(
        egui::pos2(rect.left() + 12.0, cy),
        egui::Align2::LEFT_CENTER,
        icon,
        FontId::proportional(18.0),
        if selected {
            accent_ink()
        } else if hov {
            fg()
        } else {
            text_2()
        },
    );
    p.text(
        egui::pos2(rect.left() + 40.0, cy),
        egui::Align2::LEFT_CENTER,
        label,
        medium(14.5),
        if selected || hov { fg() } else { text_2() },
    );
    if let Some(b) = badge_text {
        let galley = ui.fonts(|f| f.layout_no_wrap(b.to_string(), medium(11.5), on_accent()));
        let pad = egui::vec2(8.0, 3.0);
        let size = galley.size() + pad * 2.0;
        let r = egui::Rect::from_center_size(
            egui::pos2(rect.right() - 10.0 - size.x / 2.0, cy),
            size,
        );
        let p = ui.painter();
        p.rect_filled(r, r.height() / 2.0, accent());
        p.rect_stroke(r, r.height() / 2.0, egui::Stroke::new(1.0, fill_edge()), egui::StrokeKind::Inside);
        p.galley(r.min + pad, galley, on_accent());
    }
    resp.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// Thin determinate progress bar: yellow fill on a surface track.
pub fn progress(ui: &mut egui::Ui, frac: f32) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 6.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 3.0, surface_3());
    let mut fill = rect;
    fill.set_width(rect.width() * frac.clamp(0.0, 1.0));
    p.rect_filled(fill, 3.0, accent());
    if !is_dark() && fill.width() > 0.0 {
        p.rect_stroke(fill, 3.0, egui::Stroke::new(1.0, fill_edge()), egui::StrokeKind::Inside);
    }
}

/// 1px horizontal hairline across the available width.
pub fn hairline(ui: &mut egui::Ui) {
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 1.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, border());
}

/// The app icon (assets/icon-128.png), drawn at `size` points. Decoded once
/// per window and cached in egui's memory.
pub fn logo(ui: &mut egui::Ui, size: f32) -> egui::Response {
    let tex = logo_texture(ui.ctx());
    ui.add(egui::Image::new((tex.id(), egui::vec2(size, size))))
}

pub fn logo_texture(ctx: &egui::Context) -> egui::TextureHandle {
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
