//! Shared application styling and small code-drawn UI helpers.

use egui::{style::WidgetVisuals, Color32, CornerRadius, Stroke, Theme, Ui, Vec2};

/// Paint a compact chevron for a combo box, pointing down while closed and up while open.
/// The foreground comes from egui's current widget state so the icon follows interaction styling.
pub fn combo_box_icon(ui: &Ui, rect: egui::Rect, visuals: &WidgetVisuals, open: bool) {
    paint_chevron(
        ui,
        rect.center(),
        4.0,
        if open {
            -std::f32::consts::FRAC_PI_2
        } else {
            std::f32::consts::FRAC_PI_2
        },
        Stroke::new(visuals.fg_stroke.width.max(1.4), visuals.fg_stroke.color),
    );
}

/// Paint a disclosure chevron that rotates from right to down as the section opens.
/// Its color follows the current interactive state of the header response.
pub fn collapsing_icon(ui: &mut Ui, openness: f32, response: &egui::Response) {
    let stroke = ui.style().interact(response).fg_stroke;
    paint_chevron(
        ui,
        response.rect.center(),
        4.0,
        std::f32::consts::FRAC_PI_2 * openness.clamp(0.0, 1.0),
        Stroke::new(stroke.width.max(1.4), stroke.color),
    );
}

/// Render a custom header and an indented, animated body with an outline disclosure icon.
/// The supplied ID owns the persistent expansion state; header widgets retain their own interactions.
pub fn collapsing_section(
    ui: &mut egui::Ui,
    id: egui::Id,
    default_open: bool,
    header: impl FnOnce(&mut egui::Ui),
    body: impl FnOnce(&mut egui::Ui),
) {
    let mut state = egui::collapsing_header::CollapsingState::load_with_default_open(
        ui.ctx(),
        id,
        default_open,
    );
    let response = ui.horizontal(|ui| {
        let spacing = ui.spacing().item_spacing;
        ui.spacing_mut().item_spacing.x = 0.0;
        state.show_toggle_button(ui, collapsing_icon);
        ui.spacing_mut().item_spacing = spacing;
        header(ui);
    });
    state.show_body_indented(&response.response, ui, body);
}

fn paint_chevron(ui: &Ui, center: egui::Pos2, radius: f32, angle: f32, stroke: Stroke) {
    let (sin, cos) = angle.sin_cos();
    let rotate = |x: f32, y: f32| center + Vec2::new(x * cos - y * sin, x * sin + y * cos);
    ui.painter().line_segment(
        [rotate(-radius * 0.55, -radius), rotate(radius * 0.55, 0.0)],
        stroke,
    );
    ui.painter().line_segment(
        [rotate(radius * 0.55, 0.0), rotate(-radius * 0.55, radius)],
        stroke,
    );
}

/// Render a collapsed, contextual list of functionality restricted by the target VT.
/// The caller supplies a stable ID for its editor and already formatted entries.
pub fn vt_availability_note(
    ui: &mut Ui,
    id: impl std::hash::Hash,
    version: ag_iso_stack::object_pool::vt_version::VtVersion,
    mut entries: Vec<String>,
) {
    if entries.is_empty() {
        return;
    }
    entries.sort();
    entries.dedup();
    egui::CollapsingHeader::new(format!("Unavailable in VT{}", u8::from(version)))
        .id_salt(id)
        .icon(collapsing_icon)
        .default_open(false)
        .show(ui, |ui| {
            for entry in entries {
                ui.add(egui::Label::new(entry).wrap());
            }
        });
}

/// Semantic colors used by application surfaces and status messages.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub workspace: Color32,
    pub panel: Color32,
    pub elevated: Color32,
    pub border: Color32,
    pub text: Color32,
    pub muted_text: Color32,
    pub accent: Color32,
    pub accent_soft: Color32,
    pub warning: Color32,
    pub error: Color32,
}

/// Colors for the requested theme.
pub const fn colors(theme: Theme) -> Palette {
    match theme {
        Theme::Light => Palette {
            workspace: Color32::from_rgb(239, 241, 244),
            panel: Color32::from_rgb(250, 251, 252),
            elevated: Color32::WHITE,
            border: Color32::from_rgb(218, 222, 228),
            text: Color32::from_rgb(35, 40, 48),
            muted_text: Color32::from_rgb(105, 113, 123),
            accent: Color32::from_rgb(52, 112, 210),
            accent_soft: Color32::from_rgb(222, 235, 255),
            warning: Color32::from_rgb(173, 105, 24),
            error: Color32::from_rgb(190, 55, 55),
        },
        Theme::Dark => Palette {
            workspace: Color32::from_rgb(24, 27, 32),
            panel: Color32::from_rgb(31, 35, 41),
            elevated: Color32::from_rgb(39, 44, 51),
            border: Color32::from_rgb(57, 63, 72),
            text: Color32::from_rgb(226, 230, 236),
            muted_text: Color32::from_rgb(151, 160, 172),
            accent: Color32::from_rgb(105, 163, 255),
            accent_soft: Color32::from_rgb(43, 66, 98),
            warning: Color32::from_rgb(240, 180, 87),
            error: Color32::from_rgb(255, 112, 112),
        },
    }
}

/// The active palette for a UI region.
pub fn palette(ui: &Ui) -> Palette {
    colors(if ui.visuals().dark_mode {
        Theme::Dark
    } else {
        Theme::Light
    })
}

/// Show selectable, wrapped error text with a compact button to copy the full message.
pub fn error_text(ui: &mut Ui, message: &str) -> egui::Response {
    let color = ui.visuals().error_fg_color;
    error_text_with_color(ui, message, color)
}

/// Show selectable, wrapped status text with a compact button to copy the full message.
pub fn error_text_with_color(ui: &mut Ui, message: &str, color: Color32) -> egui::Response {
    ui.horizontal_top(|ui| {
        let response = ui.add_sized(egui::vec2(20.0, 20.0), egui::Button::new(""));
        let rect = response.rect;
        if ui.is_rect_visible(rect) {
            let stroke = ui.style().interact(&response).fg_stroke;
            let width = 8.0;
            let back = egui::Rect::from_min_size(
                rect.min + egui::vec2(6.0, 3.0),
                egui::vec2(width, width),
            );
            let front = egui::Rect::from_min_size(
                rect.min + egui::vec2(3.0, 6.0),
                egui::vec2(width, width),
            );
            ui.painter()
                .rect_stroke(back, 0.0, stroke, egui::StrokeKind::Middle);
            ui.painter()
                .rect_stroke(front, 0.0, stroke, egui::StrokeKind::Middle);
        }
        let response = response.on_hover_text("Copy error");
        if response.clicked() {
            ui.ctx().copy_text(message.to_owned());
        }
        ui.add(
            egui::Label::new(egui::RichText::new(message).color(color))
                .selectable(true)
                .wrap(),
        );
        response
    })
    .inner
}

/// Register the application light and dark styles on a context.
/// Call once during app startup; changing the selected theme then uses egui's
/// normal theme preference mechanism.
pub fn install(ctx: &egui::Context) {
    ctx.set_style_of(Theme::Light, themed_style(Theme::Light));
    ctx.set_style_of(Theme::Dark, themed_style(Theme::Dark));
}

fn themed_style(theme: Theme) -> egui::Style {
    let p = colors(theme);
    let mut style = egui::Style::default();
    style
        .text_styles
        .insert(egui::TextStyle::Body, egui::FontId::proportional(13.0));
    style
        .text_styles
        .insert(egui::TextStyle::Button, egui::FontId::proportional(13.0));
    style
        .text_styles
        .insert(egui::TextStyle::Small, egui::FontId::proportional(11.0));
    style
        .text_styles
        .insert(egui::TextStyle::Heading, egui::FontId::proportional(16.0));
    style
        .text_styles
        .insert(egui::TextStyle::Monospace, egui::FontId::monospace(12.0));
    style.spacing.item_spacing = Vec2::new(8.0, 6.0);
    style.spacing.button_padding = Vec2::new(9.0, 5.0);
    style.spacing.interact_size = Vec2::new(24.0, 26.0);
    style.spacing.window_margin = egui::Margin::same(12);
    style.visuals = if theme == Theme::Dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    let visuals = &mut style.visuals;
    visuals.panel_fill = p.panel;
    visuals.window_fill = p.panel;
    visuals.window_stroke = Stroke::new(1.0, p.border);
    visuals.window_corner_radius = CornerRadius::same(7);
    visuals.menu_corner_radius = CornerRadius::same(6);
    visuals.faint_bg_color = p.workspace;
    visuals.extreme_bg_color = p.elevated;
    visuals.text_edit_bg_color = Some(p.elevated);
    visuals.code_bg_color = p.workspace;
    visuals.warn_fg_color = p.warning;
    visuals.error_fg_color = p.error;
    visuals.hyperlink_color = p.accent;
    visuals.selection.bg_fill = p.accent_soft;
    visuals.selection.stroke = Stroke::new(1.0, p.accent);
    visuals.widgets.noninteractive.bg_fill = p.panel;
    visuals.widgets.noninteractive.bg_stroke = Stroke::new(1.0, p.border);
    visuals.widgets.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    visuals.widgets.inactive.bg_fill = p.elevated;
    visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
    visuals.widgets.inactive.bg_stroke = Stroke::new(1.0, p.border);
    visuals.widgets.inactive.fg_stroke = Stroke::new(1.0, p.text);
    visuals.widgets.inactive.corner_radius = CornerRadius::same(5);
    visuals.widgets.hovered.bg_fill = p.accent_soft;
    visuals.widgets.hovered.weak_bg_fill = p.accent_soft;
    visuals.widgets.hovered.bg_stroke = Stroke::new(1.0, p.accent);
    visuals.widgets.hovered.fg_stroke = Stroke::new(1.0, p.text);
    visuals.widgets.hovered.corner_radius = CornerRadius::same(5);
    visuals.widgets.active.bg_fill = p.accent;
    visuals.widgets.active.weak_bg_fill = p.accent_soft;
    visuals.widgets.active.bg_stroke = Stroke::new(1.0, p.accent);
    visuals.widgets.active.fg_stroke = Stroke::new(1.0, Color32::WHITE);
    visuals.widgets.active.corner_radius = CornerRadius::same(5);
    visuals.widgets.open.bg_fill = p.accent_soft;
    visuals.widgets.open.weak_bg_fill = p.accent_soft;
    visuals.widgets.open.bg_stroke = Stroke::new(1.0, p.accent);
    visuals.widgets.open.fg_stroke = Stroke::new(1.0, p.text);
    visuals.widgets.open.corner_radius = CornerRadius::same(5);
    visuals.button_frame = true;
    visuals.collapsing_header_frame = false;
    style
}

/// Render a compact section label with a subtle divider.
pub fn section_heading(ui: &mut Ui, title: &str) {
    let p = palette(ui);
    ui.add_space(5.0);
    ui.horizontal(|ui| {
        ui.label(
            egui::RichText::new(title.to_uppercase())
                .size(11.0)
                .strong()
                .color(p.muted_text),
        );
        let remaining = ui.available_width();
        if remaining > 8.0 {
            let (rect, _) = ui.allocate_exact_size(Vec2::new(remaining, 1.0), egui::Sense::hover());
            ui.painter()
                .hline(rect.x_range(), rect.center().y, Stroke::new(1.0, p.border));
        }
    });
    ui.add_space(2.0);
}

/// Small set of simple interface glyphs drawn with egui primitives.
#[derive(Clone, Copy, Debug)]
pub enum Icon {
    Plus,
    Close,
    More,
    ChevronLeft,
    ChevronRight,
    ChevronDown,
    ChevronUp,
}

/// Create a compact icon-only button with an accessible tooltip.
pub fn icon_button(ui: &mut Ui, icon: Icon, tooltip: &str) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(28.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let p = palette(ui);
        let state = ui.style().interact(&response);
        ui.painter().rect(
            rect,
            CornerRadius::same(5),
            state.bg_fill,
            state.bg_stroke,
            egui::StrokeKind::Inside,
        );
        let center = rect.center();
        let half = 5.0;
        let stroke = Stroke::new(1.6, state.fg_stroke.color);
        match icon {
            Icon::Plus => {
                ui.painter().line_segment(
                    [center - Vec2::new(half, 0.0), center + Vec2::new(half, 0.0)],
                    stroke,
                );
                ui.painter().line_segment(
                    [center - Vec2::new(0.0, half), center + Vec2::new(0.0, half)],
                    stroke,
                );
            }
            Icon::Close => {
                ui.painter().line_segment(
                    [center - Vec2::splat(half), center + Vec2::splat(half)],
                    stroke,
                );
                ui.painter().line_segment(
                    [
                        center + Vec2::new(-half, half),
                        center + Vec2::new(half, -half),
                    ],
                    stroke,
                );
            }
            Icon::More => {
                for x in [-4.0, 0.0, 4.0] {
                    ui.painter()
                        .circle_filled(center + Vec2::new(x, 0.0), 1.4, p.muted_text);
                }
            }
            Icon::ChevronLeft => {
                paint_chevron(ui, center, half, std::f32::consts::PI, stroke);
            }
            Icon::ChevronRight => {
                paint_chevron(ui, center, half, 0.0, stroke);
            }
            Icon::ChevronDown => {
                paint_chevron(ui, center, half, std::f32::consts::FRAC_PI_2, stroke);
            }
            Icon::ChevronUp => {
                paint_chevron(ui, center, half, -std::f32::consts::FRAC_PI_2, stroke);
            }
        }
    }
    response.on_hover_text(tooltip)
}
