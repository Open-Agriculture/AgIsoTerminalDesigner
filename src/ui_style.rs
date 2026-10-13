//! Shared application styling and small code-drawn UI helpers.

use egui::{Color32, CornerRadius, Stroke, Theme, Ui, Vec2};

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
                ui.painter().line_segment(
                    [
                        center + Vec2::new(2.5, -half),
                        center + Vec2::new(-2.5, 0.0),
                    ],
                    stroke,
                );
                ui.painter().line_segment(
                    [center + Vec2::new(-2.5, 0.0), center + Vec2::new(2.5, half)],
                    stroke,
                );
            }
            Icon::ChevronRight => {
                ui.painter().line_segment(
                    [
                        center + Vec2::new(-2.5, -half),
                        center + Vec2::new(2.5, 0.0),
                    ],
                    stroke,
                );
                ui.painter().line_segment(
                    [center + Vec2::new(2.5, 0.0), center + Vec2::new(-2.5, half)],
                    stroke,
                );
            }
        }
    }
    response.on_hover_text(tooltip)
}
