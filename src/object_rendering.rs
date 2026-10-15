//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

use std::{cell::RefCell, collections::HashSet, ops::Sub};

use ag_iso_stack::object_pool::object::*;
use ag_iso_stack::object_pool::object_attributes::ButtonState;
use ag_iso_stack::object_pool::object_attributes::FontSize;
use ag_iso_stack::object_pool::object_attributes::FormatType;
use ag_iso_stack::object_pool::object_attributes::HorizontalAlignment;
use ag_iso_stack::object_pool::object_attributes::LineDirection;
use ag_iso_stack::object_pool::object_attributes::Point;
use ag_iso_stack::object_pool::object_attributes::VerticalAlignment;
use ag_iso_stack::object_pool::vt_version::VtVersion;
use ag_iso_stack::object_pool::Colour;
use ag_iso_stack::object_pool::ObjectPool;
use ag_iso_stack::object_pool::ObjectRef;
use eframe::egui;
use eframe::egui::Color32;
use eframe::egui::TextWrapMode;
use eframe::egui::UiBuilder;

/// Border inset used by the preview's button rendering.
pub(crate) const BUTTON_BORDER_INSET: f32 = 4.0;
fn render_version_id() -> egui::Id {
    egui::Id::new("vt_render_version")
}

/// Render a pool object using the requested target VT version. The version is
/// scoped to this egui context for the duration of rendering, including child
/// objects. Direct legacy `RenderableObject::render` calls use VT6, the safe
/// default for the designer's historical previews.
pub fn render_object_with_version(
    ui: &mut egui::Ui,
    pool: &ObjectPool,
    object: &Object,
    position: Point<i16>,
    version: VtVersion,
) {
    let previous = ui.ctx().data_mut(|data| {
        let previous = data.get_temp::<VtVersion>(render_version_id());
        data.insert_temp(render_version_id(), version);
        previous
    });
    object.render(ui, pool, position);
    ui.ctx().data_mut(|data| match previous {
        Some(previous) => data.insert_temp(render_version_id(), previous),
        None => {
            data.remove_temp::<VtVersion>(render_version_id());
        }
    });
}

fn render_version(ui: &egui::Ui) -> VtVersion {
    ui.ctx()
        .data(|data| data.get_temp::<VtVersion>(render_version_id()))
        .unwrap_or(VtVersion::Version6)
}

thread_local! {
    /// Prevent malformed cyclic object references from recursing forever.
    static RENDER_ANCESTRY: RefCell<HashSet<u16>> = RefCell::new(HashSet::new());
}

struct RenderAncestryGuard(u16);

impl RenderAncestryGuard {
    fn enter(object_id: u16) -> Option<Self> {
        RENDER_ANCESTRY.with(|active| {
            active
                .borrow_mut()
                .insert(object_id)
                .then_some(Self(object_id))
        })
    }
}

impl Drop for RenderAncestryGuard {
    fn drop(&mut self) {
        RENDER_ANCESTRY.with(|active| {
            active.borrow_mut().remove(&self.0);
        });
    }
}

pub trait RenderableObject {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>);
}

impl RenderableObject for Object {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let Some(_ancestry_guard) = RenderAncestryGuard::enter(self.id().value()) else {
            return;
        };

        // Make sure text is truncated if it doesn't fit for all object renderings (useful for error labels)
        ui.style_mut().wrap_mode = Some(TextWrapMode::Truncate);

        let version = render_version(ui);
        if !crate::vt_compatibility::supported_object_type(self.object_type(), version) {
            ui.painter().text(
                ui.min_rect().min + egui::vec2(position.x as f32, position.y as f32),
                egui::Align2::LEFT_TOP,
                format!(
                    "{:?} is unavailable in VT{}",
                    self.object_type(),
                    u8::from(version)
                ),
                egui::FontId::proportional(12.0),
                ui.visuals().warn_fg_color,
            );
            return;
        }

        match self {
            Object::WorkingSet(o) => o.render(ui, pool, position),
            Object::DataMask(o) => o.render(ui, pool, position),
            Object::AlarmMask(o) => o.render(ui, pool, position),
            Object::Container(o) => o.render(ui, pool, position),
            Object::SoftKeyMask(o) => (),
            Object::Key(o) => o.render(ui, pool, position),
            Object::Button(o) => o.render(ui, pool, position),
            Object::InputBoolean(o) => o.render(ui, pool, position),
            Object::InputString(o) => o.render(ui, pool, position),
            Object::InputNumber(o) => o.render(ui, pool, position),
            Object::InputList(o) => o.render(ui, pool, position),
            Object::OutputString(o) => o.render(ui, pool, position),
            Object::OutputNumber(o) => o.render(ui, pool, position),
            Object::OutputList(o) => o.render(ui, pool, position),
            Object::OutputLine(o) => o.render(ui, pool, position),
            Object::OutputRectangle(o) => o.render(ui, pool, position),
            Object::OutputEllipse(o) => o.render(ui, pool, position),
            Object::OutputPolygon(o) => o.render(ui, pool, position),
            Object::OutputMeter(o) => o.render(ui, pool, position),
            Object::OutputLinearBarGraph(o) => o.render(ui, pool, position),
            Object::OutputArchedBarGraph(o) => o.render(ui, pool, position),
            Object::PictureGraphic(o) => o.render(ui, pool, position),
            Object::NumberVariable(o) => (),
            Object::StringVariable(o) => (),
            Object::FontAttributes(o) => (),
            Object::LineAttributes(o) => (),
            Object::FillAttributes(o) => (),
            Object::InputAttributes(o) => (),
            Object::ObjectPointer(o) => o.render(ui, pool, position),
            Object::Macro(o) => (),
            Object::AuxiliaryFunctionType2(o) => o.render(ui, pool, position),
            Object::AuxiliaryInputType2(o) => o.render(ui, pool, position),
            Object::AuxiliaryControlDesignatorType2(o) => o.render(ui, pool, position),
            Object::ExtendedInputAttributes(o) => (),
            Object::ColourMap(o) => (),
            Object::ObjectLabelReferenceList(o) => (),
            Object::ExternalObjectDefinition(o) => (),
            Object::ExternalReferenceName(o) => (),
            Object::ColourPalette(o) => (),
            Object::GraphicData(o) => (),
            Object::WorkingSetSpecialControls(o) => (),
            Object::AuxiliaryFunctionType1(_)
            | Object::AuxiliaryInputType1(_)
            | Object::WindowMask(_)
            | Object::KeyGroup(_)
            | Object::GraphicsContext(_)
            | Object::ExternalObjectPointer(_)
            | Object::Animation(_)
            | Object::ScaledGraphic(_) => {
                ui.painter().text(
                    ui.min_rect().min + egui::vec2(position.x as f32, position.y as f32),
                    egui::Align2::LEFT_TOP,
                    format!("Preview unavailable: {:?}", self.object_type()),
                    egui::FontId::proportional(12.0),
                    ui.visuals().warn_fg_color,
                );
            }
        }
    }
}

pub(crate) trait Colorable {
    fn convert(&self) -> egui::Color32;
}

impl Colorable for Colour {
    fn convert(&self) -> egui::Color32 {
        egui::Color32::from_rgb(self.r, self.g, self.b)
    }
}

// Helper function to lighten a color by a certain amount
fn lighten_color(color: egui::Color32, amount: f32) -> egui::Color32 {
    let r = (color.r() as f32 + 255.0 * amount).min(255.0) as u8;
    let g = (color.g() as f32 + 255.0 * amount).min(255.0) as u8;
    let b = (color.b() as f32 + 255.0 * amount).min(255.0) as u8;
    egui::Color32::from_rgb(r, g, b)
}

// Helper function to darken a color by a certain amount
fn darken_color(color: egui::Color32, amount: f32) -> egui::Color32 {
    let r = (color.r() as f32 * (1.0 - amount)).max(0.0) as u8;
    let g = (color.g() as f32 * (1.0 - amount)).max(0.0) as u8;
    let b = (color.b() as f32 * (1.0 - amount)).max(0.0) as u8;
    egui::Color32::from_rgb(r, g, b)
}

fn create_relative_rect(ui: &mut egui::Ui, position: Point<i16>, size: egui::Vec2) -> egui::Rect {
    let width = ui.max_rect().width().sub(position.x as f32).min(size.x);
    let height = ui.max_rect().height().sub(position.y as f32).min(size.y);

    egui::Rect::from_min_size(
        ui.max_rect().min + egui::vec2(position.x as f32, position.y as f32),
        egui::vec2(width, height),
    )
}

fn render_object_refs(ui: &mut egui::Ui, pool: &ObjectPool, object_refs: &Vec<ObjectRef>) {
    for object in object_refs.iter() {
        match pool.object_by_id(object.id) {
            Some(obj) => {
                obj.render(ui, pool, object.offset);
            }
            None => {
                ui.colored_label(Color32::RED, format!("Missing object: {:?}", object));
            }
        }
    }
}

fn fill_polygon_even_odd(
    painter: &egui::Painter,
    polygon_points: &[egui::Pos2],
    fill_color: Color32,
    clip_rect: egui::Rect,
) {
    if polygon_points.len() < 3 || fill_color == Color32::TRANSPARENT {
        return;
    }

    let mut min_y = f32::INFINITY;
    let mut max_y = f32::NEG_INFINITY;
    for p in polygon_points {
        min_y = min_y.min(p.y);
        max_y = max_y.max(p.y);
    }

    min_y = min_y.max(clip_rect.top());
    max_y = max_y.min(clip_rect.bottom());
    if max_y <= min_y {
        return;
    }

    let y_start = min_y.floor() as i32;
    let y_end = max_y.ceil() as i32;

    for y in y_start..y_end {
        let y_sample = y as f32 + 0.5;
        let mut x_intersections: Vec<f32> = Vec::new();

        for idx in 0..polygon_points.len() {
            let p0 = polygon_points[idx];
            let p1 = polygon_points[(idx + 1) % polygon_points.len()];

            let y0 = p0.y;
            let y1 = p1.y;

            if (y0 <= y_sample && y_sample < y1) || (y1 <= y_sample && y_sample < y0) {
                let t = (y_sample - y0) / (y1 - y0);
                let x = p0.x + t * (p1.x - p0.x);
                x_intersections.push(x);
            }
        }

        if x_intersections.len() < 2 {
            continue;
        }

        x_intersections.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));

        for pair in x_intersections.chunks_exact(2) {
            let x0 = pair[0].max(clip_rect.left());
            let x1 = pair[1].min(clip_rect.right());
            if x1 > x0 {
                painter.line_segment(
                    [egui::pos2(x0, y_sample), egui::pos2(x1, y_sample)],
                    egui::Stroke::new(1.0, fill_color),
                );
            }
        }
    }
}

impl RenderableObject for WorkingSet {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, _: Point<i16>) {
        ui.painter().rect_filled(
            ui.available_rect_before_wrap(),
            0.0,
            crate::effective_colour::colour(pool, self.background_colour).convert(),
        );

        render_object_refs(ui, pool, &self.object_refs);
    }
}

impl RenderableObject for DataMask {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, _: Point<i16>) {
        ui.painter().rect_filled(
            ui.available_rect_before_wrap(),
            0.0,
            crate::effective_colour::colour(pool, self.background_colour).convert(),
        );

        render_object_refs(ui, pool, &self.object_refs);
    }
}

impl RenderableObject for AlarmMask {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, _: Point<i16>) {
        ui.painter().rect_filled(
            ui.available_rect_before_wrap(),
            0.0,
            crate::effective_colour::colour(pool, self.background_colour).convert(),
        );

        render_object_refs(ui, pool, &self.object_refs);
    }
}

impl RenderableObject for Container {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        if self.hidden {
            return;
        }

        let rect = create_relative_rect(
            ui,
            position,
            egui::Vec2::new(self.width() as f32, self.height() as f32),
        );

        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            ui.shrink_clip_rect(rect);
            render_object_refs(ui, pool, &self.object_refs);
        });
    }
}

impl RenderableObject for Button {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let vt_version = render_version(ui);

        let rect = create_relative_rect(
            ui,
            position,
            egui::Vec2::new(self.width() as f32, self.height() as f32),
        );

        let mut no_border = false;
        let mut suppress_border = false;
        let mut transparent_background = false;
        let mut disabled = false;

        if vt_version >= VtVersion::Version4 {
            // The following attributes are only available in VT version 4 and later.
            no_border = self.options.no_border;
            suppress_border = self.options.suppress_border;
            transparent_background = self.options.transparent_background;
            disabled = self.options.disabled;
        }

        // Determine if button is latchable and currently latched (pressed).
        let latchable = self.options.latchable;
        let latched = if latchable {
            self.options.state == ButtonState::Latched
        } else {
            false
        };

        // Compute the face rectangle based on border settings
        // According to the standard:
        // - If no_border = true: Face area = entire area (no border space).
        // - If no_border = false: Face is 8 pixels smaller in width and height.
        //
        // The border is a VT proprietary 8-pixel area, but we must reduce face size accordingly.
        // Let's assume a uniform distribution of that 8-pixel shrinkage (4 pixels on each side).
        let face_rect = if no_border {
            rect
        } else {
            // Face is area minus 8 pixels in width and height.
            // We'll just evenly shrink by 4 pixels on each side.
            rect.shrink(BUTTON_BORDER_INSET)
        };

        let response = ui.interact(
            face_rect,
            ui.id().with(self.id.value()),
            egui::Sense::click(),
        );

        // Determine the current visual state
        // Priority: latched > pressed > hovered > normal
        let is_pressed_state = latched || (response.is_pointer_button_down_on() && !latchable);
        let is_hovered_state = response.hovered();
        // TODO: better visuals for latched states

        let background_color = if transparent_background {
            egui::Color32::TRANSPARENT
        } else {
            let color = crate::effective_colour::colour(pool, self.background_colour).convert();
            if is_pressed_state {
                darken_color(color, 0.2)
            } else if is_hovered_state {
                lighten_color(color, 0.1)
            } else {
                color
            }
        };

        let border_color = if suppress_border {
            egui::Color32::TRANSPARENT
        } else {
            let color = crate::effective_colour::colour(pool, self.border_colour).convert();
            if is_pressed_state {
                lighten_color(color, 0.1)
            } else if is_hovered_state {
                darken_color(color, 0.05)
            } else {
                color
            }
        };

        if !no_border {
            ui.painter().rect_stroke(
                rect,
                0.0,
                egui::Stroke::new(BUTTON_BORDER_INSET, border_color),
                egui::StrokeKind::Inside,
            );
        }

        ui.painter().rect_filled(face_rect, 0.0, background_color);

        // Child objects are clipped to the face area
        ui.scope_builder(UiBuilder::new().max_rect(face_rect), |ui| {
            ui.shrink_clip_rect(face_rect);
            render_object_refs(ui, pool, &self.object_refs);
        });

        // If disabled, we overlay a semi-transparent gray:
        if disabled {
            ui.painter().rect_filled(
                face_rect,
                0.0,
                egui::Color32::from_rgba_premultiplied(128, 128, 128, 100),
            );
        }
    }
}

impl RenderableObject for InputBoolean {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let vt_version = render_version(ui);
        let is_true = if let Some(var_id) = self.variable_reference.0 {
            match pool.object_by_id(var_id) {
                Some(Object::NumberVariable(num_var)) if vt_version >= VtVersion::Version4 => {
                    num_var.value > 0
                }
                Some(Object::NumberVariable(num_var)) => num_var.value == 1,
                _ => {
                    if vt_version >= VtVersion::Version4 {
                        self.value > 0
                    } else {
                        self.value == 1
                    }
                }
            }
        } else {
            if vt_version >= VtVersion::Version4 {
                self.value > 0
            } else {
                self.value == 1
            }
        };

        let side = self.width as f32;
        let rect = create_relative_rect(ui, position, egui::Vec2::new(side, side));

        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            let background_color =
                crate::effective_colour::colour(pool, self.background_colour).convert();
            ui.painter().rect_filled(rect, 0.0, background_color);

            // If the boolean is true, we display a checkmark in the center
            if is_true {
                let fg_color = match pool.object_by_nullable_id(self.foreground_colour) {
                    Some(Object::FontAttributes(font_attr)) => {
                        crate::effective_colour::colour(pool, font_attr.font_colour).convert()
                    }
                    // Fall back if missing or the ID is invalid.
                    _ => egui::Color32::BLACK,
                };

                let font_id = egui::FontId::new(side, egui::FontFamily::Proportional);
                ui.painter().text(
                    rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "\u{2714}",
                    font_id,
                    fg_color,
                );
            }

            // If disabled, overlay a semi-transparent layer
            if vt_version >= VtVersion::Version4 && !self.enabled {
                ui.painter().rect_filled(
                    rect,
                    0.0,
                    egui::Color32::from_rgba_premultiplied(128, 128, 128, 100),
                );
            }
        });
    }
}

impl RenderableObject for InputString {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let rect = create_relative_rect(
            ui,
            position,
            egui::Vec2::new(self.width() as f32, self.height() as f32),
        );

        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            ui.colored_label(Color32::RED, "InputString not implemented");
        });
    }
}

impl RenderableObject for InputNumber {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let rect = create_relative_rect(
            ui,
            position,
            egui::Vec2::new(self.width as f32, self.height as f32),
        );

        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            // Look up the font attributes. If missing, show an error.
            let font_attributes = match pool.object_by_nullable_id(self.font_attributes) {
                Some(Object::FontAttributes(fa)) => fa,
                _ => {
                    ui.colored_label(
                        egui::Color32::RED,
                        format!(
                            "Missing FontAttributes for InputNumber ID {:?}",
                            self.id.value()
                        ),
                    );
                    return;
                }
            };

            // Get the background colour from the pool.
            let background_colour =
                crate::effective_colour::colour(pool, self.background_colour).convert();
            // Fill the background if the NumberOptions do not specify transparency.
            if !self.options.transparent {
                ui.painter().rect_filled(rect, 0.0, background_colour);
            }

            // Determine the “raw” number value to use: if a variable_reference exists, use the referenced
            // NumberVariable’s value; otherwise use our own value.
            let raw_value: u32 = if let Some(var_id) = self.variable_reference.0 {
                match pool.object_by_id(var_id) {
                    Some(Object::NumberVariable(num_var)) => num_var.value,
                    _ => self.value,
                }
            } else {
                self.value
            };

            // Compute the displayed value using double precision:
            //   displayed_value = (raw_value + offset) * scale
            let mut displayed_value = {
                let float_raw = raw_value as f64;
                let float_offset = self.offset as f64;
                let float_scale = self.scale as f64;
                (float_raw + float_offset) * float_scale
            };

            // Use the number of decimals (up to 7) and the "truncate" flag from NumberOptions
            let decimals = self.nr_of_decimals.min(7);
            let power_of_ten = 10f64.powi(decimals as i32);
            if self.options.truncate {
                displayed_value = (displayed_value * power_of_ten).trunc() / power_of_ten;
            } else {
                displayed_value = (displayed_value * power_of_ten).round() / power_of_ten;
            }

            // If the "display_zero_as_blank" option is set and the computed value is exactly zero, show nothing.
            if self.options.display_zero_as_blank && displayed_value == 0.0 {
                return;
            }

            // Format the number to a string. Use exponential formatting if requested.
            let mut number_string = if self.format == FormatType::Exponential {
                format!("{:.*e}", decimals as usize, displayed_value)
            } else {
                format!("{:.*}", decimals as usize, displayed_value)
            };

            // If the "display_leading_zeros" option is set, try to pad the text on the left with zeros
            // so that it fills (or exceeds) the available field width.
            if self.options.display_leading_zeros {
                let fonts = ui.fonts(|f| f.clone());
                let font_height = match font_attributes.font_size {
                    FontSize::NonProportional(size) => size.height() as f32,
                    FontSize::Proportional(h) => h as f32,
                };
                let font_id = egui::FontId::new(font_height, egui::FontFamily::Proportional);
                let mut zero_padded = number_string.clone();
                let max_loop = 1000; // safety to avoid an infinite loop
                for _ in 0..max_loop {
                    let galley = fonts.layout_no_wrap(
                        zero_padded.clone(),
                        font_id.clone(),
                        crate::effective_colour::colour(pool, font_attributes.font_colour)
                            .convert(),
                    );
                    if galley.size().x >= rect.width() {
                        number_string = zero_padded;
                        break;
                    } else {
                        zero_padded.insert(0, '0');
                    }
                }
            }

            // Get the font colour.
            let font_colour =
                crate::effective_colour::colour(pool, font_attributes.font_colour).convert();

            // Choose the font family and height according to the font size:
            let (font_family, font_height) = match font_attributes.font_size {
                FontSize::NonProportional(npsize) => {
                    (egui::FontFamily::Monospace, npsize.height() as f32)
                }
                FontSize::Proportional(h) => (egui::FontFamily::Proportional, h as f32),
            };
            let font_id = egui::FontId::new(font_height, font_family);

            // Lay out the text.
            let fonts = ui.fonts(|f| f.clone());
            let galley = fonts.layout(
                number_string.clone(),
                font_id.clone(),
                font_colour,
                f32::INFINITY,
            );
            let text_size = galley.size();

            // Compute the text’s paint position according to the horizontal and vertical justification.
            let mut paint_pos = rect.min;
            match self.justification.horizontal {
                HorizontalAlignment::Left => {
                    paint_pos.x = rect.min.x;
                }
                HorizontalAlignment::Middle => {
                    paint_pos.x = rect.center().x - text_size.x * 0.5;
                }
                HorizontalAlignment::Right => {
                    paint_pos.x = rect.max.x - text_size.x;
                }
                HorizontalAlignment::Reserved => {
                    ui.colored_label(
                        egui::Color32::RED,
                        "Invalid horizontal alignment for InputNumber",
                    );
                    return;
                }
            }
            match self.justification.vertical {
                VerticalAlignment::Top => {
                    paint_pos.y = rect.min.y;
                }
                VerticalAlignment::Middle => {
                    paint_pos.y = rect.center().y - text_size.y * 0.5;
                }
                VerticalAlignment::Bottom => {
                    paint_pos.y = rect.max.y - text_size.y;
                }
                VerticalAlignment::Reserved => {
                    ui.colored_label(
                        egui::Color32::RED,
                        "Invalid vertical alignment for InputNumber",
                    );
                    return;
                }
            }

            // Draw the number string.
            ui.painter().galley(paint_pos, galley, font_colour);

            // If the InputNumber object is not enabled (according to its InputNumberOptions),
            // overlay a semi‐transparent gray rectangle.
            if !self.options2.enabled {
                ui.painter().rect_filled(
                    rect,
                    0.0,
                    egui::Color32::from_rgba_premultiplied(128, 128, 128, 100),
                );
            }
        });
    }
}

impl RenderableObject for InputList {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let rect = create_relative_rect(
            ui,
            position,
            egui::Vec2::new(self.width() as f32, self.height() as f32),
        );

        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            ui.colored_label(Color32::RED, "InputList not implemented");
        });
    }
}

impl RenderableObject for Key {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let available = ui.available_rect_before_wrap();
        let rect = create_relative_rect(ui, position, available.size());
        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            ui.shrink_clip_rect(rect);
            ui.painter().rect_filled(
                rect,
                0.0,
                crate::effective_colour::colour(pool, self.background_colour).convert(),
            );
            render_object_refs(ui, pool, &self.object_refs);
        });
    }
}

impl RenderableObject for ObjectPointer {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        if self.value.0.is_none() {
            // No object selected
            return;
        }

        match pool.object_by_id(self.value.0.unwrap()) {
            Some(obj) => {
                obj.render(ui, pool, position);
            }
            None => {
                ui.colored_label(Color32::RED, format!("Missing object: {:?}", self));
            }
        }
    }
}

impl RenderableObject for OutputString {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        crate::output_string_rendering::render(self, ui, pool, position, render_version(ui));
    }
}

impl RenderableObject for OutputNumber {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let rect = create_relative_rect(
            ui,
            position,
            egui::Vec2::new(self.width() as f32, self.height() as f32),
        );

        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            // 1. Get the font attributes
            let font_attributes = match pool.object_by_nullable_id(self.font_attributes) {
                Some(Object::FontAttributes(fa)) => fa,
                _ => {
                    ui.colored_label(
                        Color32::RED,
                        format!(
                            "Missing FontAttributes for OutputNumber ID {:?}",
                            self.id.value()
                        ),
                    );
                    return;
                }
            };

            // 2. Convert the pool color indices to `egui::Color32`
            let background_colour =
                crate::effective_colour::colour(pool, self.background_colour).convert();
            let font_colour =
                crate::effective_colour::colour(pool, font_attributes.font_colour).convert();

            // 3. Determine if we need to fill the background or remain transparent
            if !self.options.transparent {
                ui.painter().rect_filled(rect, 0.0, background_colour);
            }

            // 4. Retrieve the raw value (either from variable_reference or this object’s own `value`)
            let raw_value = if let Some(var_id) = self.variable_reference.into() {
                // If we have a referenced NumberVariable, use it
                match pool.object_by_id(var_id) {
                    Some(Object::NumberVariable(num_var)) => num_var.value,
                    _ => self.value,
                }
            } else {
                self.value
            };

            // 5. Compute the displayed value using double precision to reduce rounding errors
            let mut displayed_value = {
                let float_raw = raw_value as f64;
                let float_offset = self.offset as f64;
                let float_scale = self.scale as f64;
                (float_raw + float_offset) * float_scale
            };

            // 6. Apply truncation or rounding to the number of decimals
            let decimals = self.nr_of_decimals.min(7); // standard says 0–7 decimals
            let power_of_ten = 10f64.powi(decimals as i32);

            if self.options.truncate {
                // Truncate
                displayed_value = (displayed_value * power_of_ten).trunc() / power_of_ten;
            } else {
                // Round
                displayed_value = (displayed_value * power_of_ten).round() / power_of_ten;
            }

            // 7. If "display_zero_as_blank" and the final number is exactly zero, display blank
            //    We interpret "exactly zero" after the rounding/truncation step
            if self.options.display_zero_as_blank && displayed_value == 0.0 {
                return;
            }

            // 8. Convert the (possibly truncated/rounded) displayed_value to string
            //    Depending on the "format" attribute, use decimal or exponential
            let mut number_string = if self.format == FormatType::Exponential {
                format!("{:.*e}", decimals as usize, displayed_value)
            } else {
                format!("{:.*}", decimals as usize, displayed_value)
            };

            // 9. The standard states that we must always display at least one digit
            //    before the decimal point (i.e., "0.xxxx" if the absolute value < 1)
            //    Normal Rust formatting already ensures e.g. "0.12" for 0.12,
            //    so we usually don't need a special patch here. But we keep the note.
            //
            // 10. If display_leading_zeros is set, we *attempt* to fill the entire width
            //     with zeros to the left before applying alignment. (ISO 11783 says
            //     "fill left to width of field with zeros, then apply justification.")
            //     Below is a best-effort approach: we measure the text in a loop,
            //     and keep prepending '0' until it meets or exceeds the available width.
            //     We also place a reasonable safety limit to avoid infinite loops.
            //
            if self.options.display_leading_zeros {
                let fonts = ui.fonts(|f| f.clone());
                let font_height = match font_attributes.font_size {
                    FontSize::NonProportional(s) => s.height() as f32,
                    FontSize::Proportional(h) => h as f32,
                };
                let font_id = egui::FontId::new(font_height, egui::FontFamily::Proportional);
                let mut zero_padded = number_string.clone();
                let max_loop = 1000; // safety net to avoid infinite loops
                for _ in 0..max_loop {
                    // Measure the current galley
                    let galley = fonts.layout(
                        zero_padded.as_str().to_owned(),
                        font_id.clone(),
                        font_colour,
                        f32::INFINITY, // no wrap
                    );
                    if galley.size().x >= rect.width() {
                        // Enough zeros to fill or exceed the field width
                        number_string = zero_padded;
                        break;
                    } else {
                        zero_padded.insert(0, '0');
                    }
                }
            }

            // 11. We have the final text we want to display in `number_string`.
            //     Next, figure out the font size and alignment. This is similar
            //     to the `OutputString` example.
            let fonts = ui.fonts(|fonts| fonts.clone());
            let (font_family, font_height) = match font_attributes.font_size {
                FontSize::NonProportional(npsize) => {
                    // For simplicity, treat it as monospace
                    (egui::FontFamily::Monospace, npsize.height() as f32)
                }
                FontSize::Proportional(h) => (egui::FontFamily::Proportional, h as f32),
            };
            let font_id = egui::FontId::new(font_height, font_family);
            let galley = fonts.layout(
                number_string.clone(),
                font_id.clone(),
                font_colour,
                f32::INFINITY, // no wrapping
            );
            let text_size = galley.size();

            // 12. Determine text anchor point based on the justification bits
            let mut paint_pos = rect.min;
            match self.justification.horizontal {
                HorizontalAlignment::Left => {
                    paint_pos.x = rect.min.x;
                }
                HorizontalAlignment::Middle => {
                    paint_pos.x = rect.center().x - (text_size.x * 0.5);
                }
                HorizontalAlignment::Right => {
                    paint_pos.x = rect.max.x - text_size.x;
                }
                HorizontalAlignment::Reserved => {
                    ui.colored_label(
                        Color32::RED,
                        "Configuration incorrect: horizontal alignment is set to Reserved",
                    );
                    return;
                }
            }
            match self.justification.vertical {
                VerticalAlignment::Top => {
                    paint_pos.y = rect.min.y;
                }
                VerticalAlignment::Middle => {
                    paint_pos.y = rect.center().y - (text_size.y * 0.5);
                }
                VerticalAlignment::Bottom => {
                    paint_pos.y = rect.max.y - text_size.y;
                }
                VerticalAlignment::Reserved => {
                    ui.colored_label(
                        Color32::RED,
                        "Configuration incorrect: vertical alignment is set to Reserved",
                    );
                    return;
                }
            }

            // 13. Finally, paint the text
            ui.painter().galley(paint_pos, galley, font_colour);
        });
    }
}

impl RenderableObject for OutputList {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let rect = create_relative_rect(
            ui,
            position,
            egui::Vec2::new(self.width() as f32, self.height() as f32),
        );

        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            ui.shrink_clip_rect(rect);
            if render_version(ui) < VtVersion::Version4 {
                ui.colored_label(Color32::RED, "OutputList requires VT4 or later");
                return;
            }
            let index = self
                .variable_reference
                .0
                .and_then(|id| match pool.object_by_id(id) {
                    Some(Object::NumberVariable(variable)) => Some(variable.value),
                    _ => None,
                })
                .unwrap_or(u32::from(self.value));
            let Some(item) = usize::try_from(index)
                .ok()
                .and_then(|index| self.list_items.get(index))
                .and_then(|id| id.0)
            else {
                return;
            };
            match pool.object_by_id(item) {
                Some(Object::ObjectPointer(pointer)) => {
                    if let Some(id) = pointer.value.0 {
                        if let Some(object) = pool.object_by_id(id) {
                            object.render(ui, pool, Point::default());
                        }
                    }
                }
                Some(object) => object.render(ui, pool, Point::default()),
                None => {
                    ui.colored_label(
                        Color32::RED,
                        format!("Missing OutputList item #{}", item.value()),
                    );
                }
            };
        });
    }
}

impl RenderableObject for OutputLine {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let rect = create_relative_rect(
            ui,
            position,
            egui::Vec2::new(self.width() as f32, self.height() as f32),
        );

        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            let line_attributes = match pool.object_by_nullable_id(self.line_attributes) {
                Some(Object::LineAttributes(attr)) => attr,
                _ => {
                    // If we don't have valid line attributes, just show an error and return
                    ui.colored_label(
                        Color32::RED,
                        format!(
                            "Missing or invalid LineAttributes ID: {:?}",
                            self.line_attributes
                        ),
                    );
                    return;
                }
            };

            if line_attributes.line_width == 0 {
                return;
            }

            let colour =
                crate::effective_colour::colour(pool, line_attributes.line_colour).convert();
            let stroke_width = line_attributes.line_width as f32;
            let stroke = egui::Stroke::new(stroke_width, colour);
            // TODO: implement line art

            let (start, end) = match self.line_direction {
                LineDirection::TopLeftToBottomRight => {
                    let start = rect.min;
                    let mut end = rect.max - egui::vec2(stroke_width, stroke_width);

                    // Clamp end to start
                    if end.x < start.x {
                        end.x = start.x;
                    }
                    if end.y < start.y {
                        end.y = start.y;
                    }

                    (start, end)
                }
                LineDirection::BottomLeftToTopRight => {
                    let mut start = egui::pos2(rect.left(), rect.bottom() + stroke_width);
                    let mut end = egui::pos2(rect.right() - stroke_width, rect.top());

                    // Clamping start and end
                    if end.x < start.x {
                        end.x = start.x;
                    }
                    if start.y < end.y {
                        start.y = end.y;
                    }

                    (start, end)
                }
            };

            ui.painter().line_segment([start, end], stroke);
        });
    }
}

impl RenderableObject for OutputRectangle {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let rect = create_relative_rect(
            ui,
            position,
            egui::Vec2::new(self.width() as f32, self.height() as f32),
        );

        // Paint the border of the rectangle
        let line_attributes = match pool.object_by_nullable_id(self.line_attributes) {
            Some(Object::LineAttributes(l)) => l,
            _ => {
                ui.colored_label(
                    Color32::RED,
                    format!("Missing line attributes: {:?}", self.line_attributes),
                );
                return;
            }
        };
        // Paint the fill of the rectangle
        if let Some(fill) = self.fill_attributes.into() {
            let fill_attributes = match pool.object_by_id(fill) {
                Some(Object::FillAttributes(f)) => f,
                _ => {
                    ui.colored_label(Color32::RED, format!("Missing fill attributes: {:?}", fill));
                    return;
                }
            };
            ui.painter().rect_filled(
                rect,
                0.0,
                crate::effective_colour::colour(pool, fill_attributes.fill_colour).convert(),
            );
            // TODO: implement fill type for infill
            // TODO: implement fill pattern for infill
        }

        ui.painter().rect_stroke(
            rect,
            0.0,
            egui::Stroke::new(
                line_attributes.line_width,
                crate::effective_colour::colour(pool, line_attributes.line_colour).convert(),
            ),
            egui::StrokeKind::Inside,
        );
        // TODO: implement line art for border
    }
}

impl RenderableObject for OutputEllipse {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let rect = create_relative_rect(
            ui,
            position,
            egui::Vec2::new(self.width() as f32, self.height() as f32),
        );

        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            ui.colored_label(Color32::RED, "OutputEllipse not implemented");
        });
    }
}

impl RenderableObject for OutputPolygon {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let rect = create_relative_rect(
            ui,
            position,
            egui::Vec2::new(self.width() as f32, self.height() as f32),
        );

        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            let line_attributes = match pool.object_by_nullable_id(self.line_attributes) {
                Some(Object::LineAttributes(attr)) => attr,
                _ => {
                    ui.colored_label(
                        Color32::RED,
                        format!(
                            "Missing or invalid LineAttributes ID: {:?}",
                            self.line_attributes
                        ),
                    );
                    return;
                }
            };

            if self.points.len() < 2 {
                return;
            }

            let points: Vec<egui::Pos2> = self
                .points
                .iter()
                .map(|point| rect.min + egui::vec2(point.x as f32, point.y as f32))
                .collect();

            let is_open_polygon = self.polygon_type == 3;

            if !is_open_polygon && self.points.len() >= 3 {
                if let Some(fill_id) = self.fill_attributes.into() {
                    let fill_attributes = match pool.object_by_id(fill_id) {
                        Some(Object::FillAttributes(f)) => f,
                        _ => {
                            ui.colored_label(
                                Color32::RED,
                                format!("Missing fill attributes: {:?}", fill_id),
                            );
                            return;
                        }
                    };

                    fill_polygon_even_odd(
                        ui.painter(),
                        &points,
                        crate::effective_colour::colour(pool, fill_attributes.fill_colour)
                            .convert(),
                        rect,
                    );
                    // TODO: implement fill type for infill
                    // TODO: implement fill pattern for infill
                }
            }

            if line_attributes.line_width == 0 {
                return;
            }

            let stroke = egui::Stroke::new(
                line_attributes.line_width,
                crate::effective_colour::colour(pool, line_attributes.line_colour).convert(),
            );

            if is_open_polygon {
                ui.painter().add(egui::Shape::line(points, stroke));
            } else {
                ui.painter().add(egui::Shape::closed_line(points, stroke));
            }
            // TODO: implement line art for border
        });
    }
}

impl RenderableObject for OutputMeter {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let rect = create_relative_rect(
            ui,
            position,
            egui::Vec2::new(self.width() as f32, self.height() as f32),
        );

        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            ui.colored_label(Color32::RED, "OutputMeter not implemented");
        });
    }
}

impl RenderableObject for OutputLinearBarGraph {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let rect = create_relative_rect(
            ui,
            position,
            egui::Vec2::new(self.width() as f32, self.height() as f32),
        );

        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            ui.colored_label(Color32::RED, "OutputLinearBarGraph not implemented");
        });
    }
}

impl RenderableObject for OutputArchedBarGraph {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let rect = create_relative_rect(
            ui,
            position,
            egui::Vec2::new(self.width() as f32, self.height() as f32),
        );

        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            ui.colored_label(Color32::RED, "OutputArchedBarGraph not implemented");
        });
    }
}

impl RenderableObject for PictureGraphic {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let rect = create_relative_rect(
            ui,
            position,
            egui::Vec2::new(self.width() as f32, self.height() as f32),
        );

        let texture = crate::graphics::preview::picture_texture(ui.ctx(), self, pool);

        // Use image dimensions, but clip to the available rect
        let image_size = egui::Vec2::new(self.width as f32, self.height() as f32);
        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            if let Some(texture) = texture {
                ui.image((texture.id(), image_size));
            } else {
                ui.colored_label(Color32::RED, "Failed to load image");
            }
        });
    }
}

impl RenderableObject for AuxiliaryFunctionType2 {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let rect = create_relative_rect(ui, position, egui::Vec2::INFINITY); // TODO: set size to softkey area

        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            // Draw background colour
            let background_colour =
                crate::effective_colour::colour(pool, self.background_colour).convert();
            ui.painter().rect_filled(rect, 0.0, background_colour);

            // Render child objects (the function designator content)
            render_object_refs(ui, pool, &self.object_refs);
        });
    }
}

impl RenderableObject for AuxiliaryInputType2 {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let rect = create_relative_rect(ui, position, egui::Vec2::INFINITY); // TODO: set size to softkey area

        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            // Draw background colour
            let background_colour =
                crate::effective_colour::colour(pool, self.background_colour).convert();
            ui.painter().rect_filled(rect, 0.0, background_colour);

            // Render child objects (the input designator content)
            render_object_refs(ui, pool, &self.object_refs);
        });
    }
}

impl RenderableObject for AuxiliaryControlDesignatorType2 {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let rect = create_relative_rect(ui, position, egui::Vec2::INFINITY); // TODO: set size to softkey area

        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            // Determine what to display based on pointer_type
            match self.pointer_type {
                0 => {
                    // Pointer type 0: Points to Auxiliary Object referenced by auxiliary_object_id
                    // Display the auxiliary object designator
                    if let Some(aux_obj_id) = self.auxiliary_object_id.0 {
                        match pool.object_by_id(aux_obj_id) {
                            Some(Object::AuxiliaryFunctionType2(aux_func)) => {
                                // Render the auxiliary function designator
                                render_object_refs(ui, pool, &aux_func.object_refs);
                            }
                            Some(Object::AuxiliaryInputType2(aux_input)) => {
                                // Render the auxiliary input designator
                                render_object_refs(ui, pool, &aux_input.object_refs);
                            }
                            Some(_) => {
                                ui.colored_label(
                                    Color32::RED,
                                    format!("Invalid aux object type: {:?}", aux_obj_id),
                                );
                            }
                            None => {
                                ui.colored_label(
                                    Color32::RED,
                                    format!("Missing aux object: {:?}", aux_obj_id),
                                );
                            }
                        }
                    }
                }
                1 => {
                    // Pointer type 1: Points to Auxiliary Function/Input that is assigned to the auxiliary object
                    // In a real VT, this would show the assigned auxiliary object designator.
                    // Since we don't have runtime assignment info, we show a placeholder.
                    ui.painter()
                        .rect_filled(rect, 0.0, Color32::from_rgb(220, 220, 255));
                    ui.colored_label(Color32::DARK_BLUE, "Assigned\nAux Obj");
                }
                2 => {
                    // Pointer type 2: Points to Working Set object for the owner of this pointer
                    if let Some(ws) = pool.objects().iter().find_map(|obj| {
                        if let Object::WorkingSet(ws) = obj {
                            Some(ws)
                        } else {
                            None
                        }
                    }) {
                        // Render working set designator
                        ws.render(ui, pool, Point { x: 0, y: 0 });
                    } else {
                        ui.colored_label(Color32::RED, "No working set object defined");
                    }
                }
                3 => {
                    // Pointer type 3: Points to Working Set object for the WS that owns the assigned aux object
                    // In a real VT, this would show the working set designator of the assigned object's owner.
                    // Since we don't have runtime assignment info, we show a placeholder.
                    ui.painter()
                        .rect_filled(rect, 0.0, Color32::from_rgb(255, 220, 220));
                    ui.colored_label(Color32::DARK_RED, "Assigned\nWS");
                }
                _ => {
                    // Invalid pointer type
                    ui.colored_label(
                        Color32::RED,
                        format!("Invalid pointer type: {}", self.pointer_type),
                    );
                }
            }
        });
    }
}
