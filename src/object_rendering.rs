//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

use std::{cell::RefCell, collections::HashSet};

use ag_iso_stack::object_pool::object::*;
use ag_iso_stack::object_pool::object_attributes::ButtonState;
use ag_iso_stack::object_pool::object_attributes::FormatType;
use ag_iso_stack::object_pool::object_attributes::LineDirection;
use ag_iso_stack::object_pool::object_attributes::Point;
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
    let origin_key = egui::Id::new("vt_pattern_origin");
    let previous_origin = ui.ctx().data_mut(|data| {
        let previous = data.get_temp::<egui::Pos2>(origin_key);
        data.insert_temp(origin_key, ui.max_rect().min);
        previous
    });
    object.render(ui, pool, position);
    ui.ctx().data_mut(|data| {
        if let Some(previous) = previous_origin {
            data.insert_temp(origin_key, previous);
        } else {
            data.remove_temp::<egui::Pos2>(origin_key);
        }
    });
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
    egui::Rect::from_min_size(
        ui.max_rect().min + egui::vec2(position.x as f32, position.y as f32),
        size,
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
        // B.8.3 shares Output String formatting, including variable contents,
        // character sets, control characters, wrapping and justification.
        let output = OutputString {
            id: self.id,
            width: self.width,
            height: self.height,
            background_colour: self.background_colour,
            font_attributes: self.font_attributes,
            options: ag_iso_stack::object_pool::object_attributes::OutputStringOptions {
                transparent: self.options.transparent,
                auto_wrap: self.options.auto_wrap,
                wrap_on_hyphen: self.options.wrap_on_hyphen,
            },
            variable_reference: self.variable_reference,
            justification: self.justification,
            value: self.value.clone(),
            macro_refs: Vec::new(),
        };
        output.render(ui, pool, position);
        if !self.enabled {
            let rect = create_relative_rect(
                ui,
                position,
                egui::vec2(self.width as f32, self.height as f32),
            );
            ui.painter()
                .with_clip_rect(rect.intersect(ui.clip_rect()))
                .rect_filled(
                    rect,
                    0.0,
                    Color32::from_rgba_premultiplied(128, 128, 128, 100),
                );
        }
    }
}

impl RenderableObject for InputNumber {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let output = OutputNumber {
            id: self.id,
            width: self.width,
            height: self.height,
            background_colour: self.background_colour,
            font_attributes: self.font_attributes,
            options: self.options,
            variable_reference: self.variable_reference,
            value: self.value,
            offset: self.offset,
            scale: self.scale,
            nr_of_decimals: self.nr_of_decimals,
            format: self.format,
            justification: self.justification,
            macro_refs: Vec::new(),
        };
        output.render(ui, pool, position);
        if !self.options2.enabled {
            let rect = create_relative_rect(
                ui,
                position,
                egui::vec2(self.width as f32, self.height as f32),
            );
            ui.painter()
                .with_clip_rect(ui.clip_rect().intersect(rect))
                .rect_filled(
                    rect,
                    0.0,
                    Color32::from_rgba_premultiplied(128, 128, 128, 100),
                );
        }
    }
}

impl RenderableObject for InputList {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let rect = create_relative_rect(
            ui,
            position,
            egui::vec2(self.width as f32, self.height as f32),
        );
        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            ui.shrink_clip_rect(rect);
            render_list_item(
                ui,
                pool,
                self.variable_reference,
                self.value,
                &self.list_items,
            );
            if !self.options.enabled {
                ui.painter().rect_filled(
                    rect,
                    0.0,
                    Color32::from_rgba_premultiplied(128, 128, 128, 100),
                );
            }
        });
    }
}

fn render_list_item(
    ui: &mut egui::Ui,
    pool: &ObjectPool,
    variable: ag_iso_stack::object_pool::NullableObjectId,
    value: u8,
    items: &[ag_iso_stack::object_pool::NullableObjectId],
) {
    let index = variable
        .0
        .and_then(|id| match pool.object_by_id(id) {
            Some(Object::NumberVariable(variable)) => Some(variable.value),
            _ => None,
        })
        .unwrap_or(u32::from(value));
    // ISO 11783-6 B.8.5: 255 explicitly means no selection.
    if index == 255 {
        return;
    }
    if let Some(id) = items.get(index as usize).and_then(|item| item.0) {
        if let Some(object) = pool.object_by_id(id) {
            object.render(ui, pool, Point::default());
        }
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

/// ISO 11783-6 B.8.4/B.9.3: apply the signed offset before scaling.
fn format_number(
    raw: u32,
    offset: i32,
    scale: f32,
    decimals: u8,
    format: FormatType,
    options: ag_iso_stack::object_pool::object_attributes::NumberOptions,
) -> String {
    let value = (f64::from(raw) + f64::from(offset)) * f64::from(scale);
    if !value.is_finite() {
        return value.to_string();
    }
    if options.display_zero_as_blank && value == 0.0 {
        return String::new();
    }
    let decimals = decimals.min(7) as usize;
    if format == FormatType::Exponential {
        // Decimal places apply to the mantissa, not the unnormalized value.
        let exponent = if value == 0.0 {
            0
        } else {
            value.abs().log10().floor() as i32
        };
        let power = 10f64.powi(exponent);
        let mut mantissa = value / power;
        let precision = 10f64.powi(decimals as i32);
        mantissa = if options.truncate {
            (mantissa * precision).trunc() / precision
        } else {
            (mantissa * precision).round() / precision
        };
        let mut exponent = exponent;
        if mantissa.abs() >= 10.0 {
            mantissa /= 10.0;
            exponent += 1;
        }
        format!("{mantissa:.decimals$}E{exponent:+03}")
    } else {
        let precision = 10f64.powi(decimals as i32);
        let value = if options.truncate {
            (value * precision).trunc() / precision
        } else {
            (value * precision).round() / precision
        };
        let value = if value == 0.0 { 0.0 } else { value };
        format!("{value:.decimals$}")
    }
}

impl RenderableObject for OutputNumber {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let raw = self
            .variable_reference
            .0
            .and_then(|id| match pool.object_by_id(id) {
                Some(Object::NumberVariable(variable)) => Some(variable.value),
                _ => None,
            })
            .unwrap_or(self.value);
        let mut value = format_number(
            raw,
            self.offset,
            self.scale,
            self.nr_of_decimals,
            self.format,
            self.options,
        );
        if self.options.display_leading_zeros && !value.is_empty() {
            if let Some(Object::FontAttributes(font)) =
                pool.object_by_nullable_id(self.font_attributes)
            {
                // Use the same cell advances as the text renderer; only complete
                // leading characters that fit in the field are added.
                let measure =
                    |text: &str| crate::output_string_rendering::text_width(ui, font, text);
                let zero_width = measure("0");
                if zero_width > 0.0 {
                    let count = ((self.width as f32 - measure(&value)) / zero_width)
                        .floor()
                        .max(0.0) as usize;
                    value.insert_str(usize::from(value.starts_with('-')), &"0".repeat(count));
                }
            }
        }
        let output = OutputString {
            id: self.id,
            width: self.width,
            height: self.height,
            background_colour: self.background_colour,
            font_attributes: self.font_attributes,
            options: ag_iso_stack::object_pool::object_attributes::OutputStringOptions {
                transparent: self.options.transparent,
                auto_wrap: false,
                wrap_on_hyphen: false,
            },
            variable_reference: ag_iso_stack::object_pool::NullableObjectId(None),
            justification: self.justification,
            value: value.into(),
            macro_refs: Vec::new(),
        };
        output.render(ui, pool, position);
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
            render_list_item(
                ui,
                pool,
                self.variable_reference,
                self.value,
                &self.list_items,
            );
        });
    }
}

impl RenderableObject for OutputLine {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let rect = create_relative_rect(
            ui,
            position,
            egui::vec2(self.width as f32, self.height as f32),
        );
        let Some(Object::LineAttributes(line)) = pool.object_by_nullable_id(self.line_attributes)
        else {
            return;
        };
        let width = f32::from(line.line_width);
        // Table B.27 coordinates locate the top-left corner of the square brush.
        let right = (rect.right() - width).max(rect.left());
        let bottom = (rect.bottom() - width).max(rect.top());
        let (start, end) = match self.line_direction {
            LineDirection::TopLeftToBottomRight => (rect.min, egui::pos2(right, bottom)),
            LineDirection::BottomLeftToTopRight => (
                egui::pos2(rect.left(), bottom),
                egui::pos2(right, rect.top()),
            ),
        };
        let offset = egui::vec2(width * 0.5, width * 0.5);
        paint_shape(
            ui,
            pool,
            rect.intersect(ui.clip_rect()),
            &[start + offset, end + offset],
            line,
            None,
            0,
        );
    }
}

impl RenderableObject for OutputRectangle {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let rect = create_relative_rect(
            ui,
            position,
            egui::vec2(self.width as f32, self.height as f32),
        );
        let Some(Object::LineAttributes(line)) = pool.object_by_nullable_id(self.line_attributes)
        else {
            return;
        };
        let width = f32::from(line.line_width);
        let offset = egui::vec2(width * 0.5, width * 0.5);
        let start = rect.min + offset;
        let end = egui::pos2(
            (rect.right() - width).max(rect.left()),
            (rect.bottom() - width).max(rect.top()),
        ) + offset;
        let points = [
            start,
            egui::pos2(end.x, start.y),
            end,
            egui::pos2(start.x, end.y),
            start,
        ];
        paint_shape(
            ui,
            pool,
            rect.intersect(ui.clip_rect()),
            &points,
            line,
            pool.object_by_nullable_id(self.fill_attributes),
            self.line_suppression,
        );
    }
}

/// Paint square-brush borders and even-odd interiors. All border footprints,
/// including skipped and suppressed strokes, are excluded from the fill.
fn paint_shape(
    ui: &egui::Ui,
    pool: &ObjectPool,
    clip: egui::Rect,
    points: &[egui::Pos2],
    line: &LineAttributes,
    fill: Option<&Object>,
    suppressed_edges: u8,
) {
    if !clip.is_positive() {
        return;
    }
    let painter = ui.painter().with_clip_rect(clip);
    let line_colour = crate::effective_colour::colour(pool, line.line_colour).convert();
    let y_start = clip.top().floor() as i32;
    let y_end = clip.bottom().ceil() as i32;
    // Keep all brush footprints, including skipped line-art bits, so fill
    // never leaks into the border (4.6.21). Restrict work to visible rows.
    let mut borders = vec![Vec::<(f32, f32, bool)>::new(); (y_end - y_start) as usize];
    let width = f32::from(line.line_width);
    let mut distance = 0.0;
    if width > 0.0 {
        for (edge_index, pair) in points.windows(2).enumerate() {
            let delta = pair[1] - pair[0];
            let length = delta.length();
            if !egui::Rect::from_two_pos(pair[0], pair[1])
                .expand(width)
                .intersects(clip)
            {
                distance += length;
                continue;
            }
            let edge_visible = suppressed_edges & (1 << (edge_index % 4)) == 0;
            let mut stamp = |point: egui::Pos2, visible: bool| {
                let brush = egui::Rect::from_center_size(point, egui::vec2(width, width));
                for y in (brush.top().floor() as i32).max(y_start)
                    ..(brush.bottom().ceil() as i32).min(y_end)
                {
                    if y as f32 + 0.5 >= brush.top() && (y as f32 + 0.5) < brush.bottom() {
                        borders[(y - y_start) as usize].push((
                            brush.left(),
                            brush.right(),
                            visible,
                        ));
                    }
                }
            };
            // The hypothetical solid border bounds the fill even for skipped
            // line-art spots. Dense sampling also keeps solid curves smooth.
            let count = (length / 0.75).ceil().max(1.0) as usize;
            for i in 0..=count {
                stamp(
                    pair[0] + delta * (i as f32 / count as f32),
                    edge_visible && line.line_art == 0xffff,
                );
            }
            if edge_visible && line.line_art != 0xffff {
                // Each bit is one square brush spot, not a swept segment.
                // Carry the pattern phase across edges (4.6.21).
                let first = (distance / width).ceil() as usize;
                let last = ((distance + length) / width).floor() as usize;
                for spot in first..=last {
                    if line.line_art & (0x8000 >> (spot % 16)) != 0 {
                        let t = if length > 0.0 {
                            (spot as f32 * width - distance) / length
                        } else {
                            0.0
                        };
                        stamp(pair[0] + delta * t, true);
                    }
                }
            }
            distance += length;
        }
    }

    let pattern = match fill {
        Some(Object::FillAttributes(fill)) if fill.fill_type == 3 => {
            if let Some(Object::PictureGraphic(picture)) =
                pool.object_by_nullable_id(fill.fill_pattern)
            {
                if picture.actual_width > 0 && picture.actual_height > 0 {
                    let mut picture = picture.clone();
                    picture.options.transparent = false;
                    Some(crate::graphics::preview::decode_picture(&picture, pool))
                } else {
                    None
                }
            } else {
                None
            }
        }
        _ => None,
    };
    let solid = match fill {
        Some(Object::FillAttributes(fill)) if fill.fill_type == 1 => Some(line_colour),
        Some(Object::FillAttributes(fill)) if fill.fill_type == 2 => {
            Some(crate::effective_colour::colour(pool, fill.fill_colour).convert())
        }
        _ => None,
    };
    let anchor = ui
        .ctx()
        .data(|data| data.get_temp::<egui::Pos2>(egui::Id::new("vt_pattern_origin")))
        .unwrap_or(egui::Pos2::ZERO);
    for y in y_start..y_end {
        let row = &borders[(y - y_start) as usize];
        if solid.is_some() || pattern.is_some() {
            let sample_y = y as f32 + 0.5;
            let mut crossings = Vec::new();
            for edge in points.windows(2) {
                let (a, b) = (edge[0], edge[1]);
                if (a.y <= sample_y && sample_y < b.y) || (b.y <= sample_y && sample_y < a.y) {
                    crossings.push(a.x + (sample_y - a.y) * (b.x - a.x) / (b.y - a.y));
                }
            }
            crossings.sort_by(f32::total_cmp);
            for interval in crossings.chunks_exact(2) {
                let mut cuts: Vec<_> = row.iter().map(|(a, b, _)| (*a, *b)).collect();
                cuts.sort_by(|a, b| a.0.total_cmp(&b.0));
                let mut left = interval[0].max(clip.left());
                let right = interval[1].min(clip.right());
                cuts.push((right, right));
                for (a, b) in cuts {
                    let end = a.min(right);
                    if end > left {
                        if let Some(colour) = solid {
                            painter.rect_filled(
                                egui::Rect::from_min_max(
                                    egui::pos2(left, y as f32),
                                    egui::pos2(end, y as f32 + 1.0),
                                ),
                                0.0,
                                colour,
                            );
                        } else if let Some(pattern) = &pattern {
                            let py = ((y as f32 - anchor.y).floor() as i64)
                                .rem_euclid(pattern.size[1] as i64)
                                as usize;
                            let mut x = left.ceil() as i32;
                            let stop = end.ceil() as i32;
                            while x < stop {
                                let px = ((x as f32 - anchor.x).floor() as i64)
                                    .rem_euclid(pattern.size[0] as i64)
                                    as usize;
                                let colour = pattern.pixels[py * pattern.size[0] + px];
                                let start = x;
                                x += 1;
                                while x < stop {
                                    let px = ((x as f32 - anchor.x).floor() as i64)
                                        .rem_euclid(pattern.size[0] as i64)
                                        as usize;
                                    if pattern.pixels[py * pattern.size[0] + px] != colour {
                                        break;
                                    }
                                    x += 1;
                                }
                                painter.rect_filled(
                                    egui::Rect::from_min_size(
                                        egui::pos2(start as f32, y as f32),
                                        egui::vec2((x - start) as f32, 1.0),
                                    ),
                                    0.0,
                                    colour,
                                );
                            }
                        }
                    }
                    left = left.max(b);
                    if left >= right {
                        break;
                    }
                }
            }
        }
        // Coalesce brush footprints into runs to keep the shape count small.
        let mut visible: Vec<_> = row
            .iter()
            .filter(|(_, _, visible)| *visible)
            .map(|(a, b, _)| (*a, *b))
            .collect();
        visible.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut run: Option<(f32, f32)> = None;
        for (a, b) in visible
            .into_iter()
            .chain(std::iter::once((f32::INFINITY, f32::INFINITY)))
        {
            match run {
                Some((left, right)) if a <= right => run = Some((left, right.max(b))),
                Some((left, right)) => {
                    painter.rect_filled(
                        egui::Rect::from_min_max(
                            egui::pos2(left, y as f32),
                            egui::pos2(right, y as f32 + 1.0),
                        ),
                        0.0,
                        line_colour,
                    );
                    run = Some((a, b));
                }
                None => run = Some((a, b)),
            }
        }
    }
}

/// Ray/ellipse intersection: ISO angles are geometric angles, not the
/// parameter angle of a stretched circle (Figure B.8).
fn ellipse_point(rect: egui::Rect, angle: f32) -> egui::Pos2 {
    let radius = rect.size() * 0.5;
    let direction = egui::vec2(angle.cos(), -angle.sin());
    let distance =
        1.0 / ((direction.x / radius.x).powi(2) + (direction.y / radius.y).powi(2)).sqrt();
    rect.center() + direction * distance
}

fn ellipse_path(rect: egui::Rect, kind: u8, start: u8, end: u8) -> Vec<egui::Pos2> {
    let full = kind == 0 || start == end;
    let start_angle = if kind == 0 {
        0.0
    } else {
        (f32::from(start) * 2.0).to_radians()
    };
    let sweep = if full {
        std::f32::consts::TAU
    } else {
        ((f32::from(end) - f32::from(start)) * 2.0)
            .to_radians()
            .rem_euclid(std::f32::consts::TAU)
    };
    let steps = ((rect.width() + rect.height()) * sweep)
        .ceil()
        .clamp(8.0, 8192.0) as usize;
    let mut points: Vec<_> = (0..=steps)
        .map(|i| ellipse_point(rect, start_angle + sweep * i as f32 / steps as f32))
        .collect();
    if kind == 2 && !full {
        points.push(rect.center());
    }
    if kind != 1 || full {
        points.push(points[0]);
    }
    // Table B.31 note 2 additionally draws the radius for an equal-angle segment.
    if kind == 2 && start == end {
        points.push(rect.center());
        points.push(points[0]);
    }
    points
}

impl RenderableObject for OutputEllipse {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let rect = create_relative_rect(
            ui,
            position,
            egui::vec2(self.width as f32, self.height as f32),
        );
        let clip = rect.intersect(ui.clip_rect());
        if !clip.is_positive() || self.ellipse_type > 3 {
            return;
        }
        let Some(Object::LineAttributes(line)) = pool.object_by_nullable_id(self.line_attributes)
        else {
            ui.painter().text(
                rect.min,
                egui::Align2::LEFT_TOP,
                format!("OutputEllipse #{}: missing LineAttributes", self.id.value()),
                egui::FontId::monospace(10.0),
                Color32::RED,
            );
            return;
        };
        // Output shape dimensions include the complete square brush footprint.
        let inset = (f32::from(line.line_width) * 0.5)
            .min((rect.width().min(rect.height()) * 0.5 - 0.001).max(0.0));
        let points = ellipse_path(
            rect.shrink(inset),
            self.ellipse_type,
            self.start_angle,
            self.end_angle,
        );
        let closed = self.ellipse_type != 1 || self.start_angle == self.end_angle;
        paint_shape(
            ui,
            pool,
            clip,
            &points,
            line,
            if closed {
                pool.object_by_nullable_id(self.fill_attributes)
            } else {
                None
            },
            0,
        );
    }
}

impl RenderableObject for OutputPolygon {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let rect = create_relative_rect(
            ui,
            position,
            egui::vec2(self.width as f32, self.height as f32),
        );
        let Some(Object::LineAttributes(line)) = pool.object_by_nullable_id(self.line_attributes)
        else {
            return;
        };
        if self.points.len() < 2 {
            return;
        }
        let mut points: Vec<_> = self
            .points
            .iter()
            .map(|point| rect.min + egui::vec2(point.x as f32, point.y as f32))
            .collect();
        let closed = self.polygon_type != 3;
        if closed {
            points.push(points[0]);
        }
        paint_shape(
            ui,
            pool,
            rect.intersect(ui.clip_rect()),
            &points,
            line,
            if closed {
                pool.object_by_nullable_id(self.fill_attributes)
            } else {
                None
            },
            0,
        );
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

#[cfg(test)]
mod rendering_tests {
    use super::*;
    use ag_iso_stack::object_pool::object_attributes::NumberOptions;
    use ag_iso_stack::object_pool::{NullableObjectId, ObjectId, ObjectType};

    fn options() -> NumberOptions {
        NumberOptions {
            transparent: false,
            display_leading_zeros: false,
            display_zero_as_blank: false,
            truncate: false,
        }
    }

    #[test]
    fn numbers_apply_signed_offset_before_scale_and_format_mantissa() {
        assert_eq!(
            format_number(20, -30, 0.5, 2, FormatType::Decimal, options()),
            "-5.00"
        );
        assert_eq!(
            format_number(u32::MAX, i32::MIN, 1.0, 0, FormatType::Decimal, options()),
            "2147483647"
        );
        assert_eq!(
            format_number(12345, 0, 1.0, 2, FormatType::Exponential, options()),
            "1.23E+04"
        );
        assert_eq!(
            format_number(9999, 0, 1.0, 2, FormatType::Exponential, options()),
            "1.00E+04"
        );
        assert_eq!(
            format_number(1, 0, 0.125, 2, FormatType::Exponential, options()),
            "1.25E-01"
        );
        let mut truncate = options();
        truncate.truncate = true;
        assert_eq!(
            format_number(12399, 0, 1.0, 2, FormatType::Exponential, truncate),
            "1.23E+04"
        );
        assert_eq!(
            format_number(125, -250, 0.125, 2, FormatType::Decimal, truncate),
            "-15.62"
        );
        let mut blank = options();
        blank.display_zero_as_blank = true;
        assert_eq!(
            format_number(10, -10, 2.0, 2, FormatType::Decimal, blank),
            ""
        );
        assert_eq!(
            format_number(1, 0, 0.125, 0, FormatType::Decimal, blank),
            "0"
        );
    }

    #[test]
    fn ellipse_angles_match_figure_b8_even_when_not_circular() {
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(120.0, 60.0));
        for degrees in [45.0_f32, 90.0, 315.0] {
            let p = ellipse_point(rect, degrees.to_radians());
            let delta = p - rect.center();
            let measured = (-delta.y).atan2(delta.x).to_degrees().rem_euclid(360.0);
            assert!((measured - degrees).abs() < 0.001);
            assert!(((delta.x / 60.0).powi(2) + (delta.y / 30.0).powi(2) - 1.0).abs() < 0.001);
        }
        let sector = ellipse_path(rect, 2, 20, 100);
        assert!(sector.contains(&rect.center()));
        assert_eq!(sector.first(), sector.last());
        let chord = ellipse_path(rect, 3, 20, 100);
        assert!(!chord.contains(&rect.center()));
        assert_eq!(chord.first(), chord.last());
        let open = ellipse_path(rect, 1, 20, 100);
        assert_ne!(open.first(), open.last());
        assert!(ellipse_path(rect, 2, 20, 20).contains(&rect.center()));
    }

    #[test]
    fn input_list_uses_variable_index_and_blanks_null_and_invalid_choices() {
        let mut pool = ObjectPool::new();
        let mut child = crate::default_object(ObjectType::OutputRectangle);
        *child.mut_id() = ObjectId::new(10).unwrap();
        let Object::OutputRectangle(rectangle) = &mut child else {
            unreachable!()
        };
        rectangle.width = 10;
        rectangle.height = 10;
        rectangle.line_attributes = ObjectId::new(11).unwrap().into();
        pool.add(child);
        let mut line = crate::default_object(ObjectType::LineAttributes);
        *line.mut_id() = ObjectId::new(11).unwrap();
        if let Object::LineAttributes(line) = &mut line {
            line.line_width = 1;
            line.line_art = 0xffff;
        }

        pool.add(line);
        let mut variable = crate::default_object(ObjectType::NumberVariable);
        *variable.mut_id() = ObjectId::new(12).unwrap();
        if let Object::NumberVariable(v) = &mut variable {
            v.value = 1;
        }
        pool.add(variable);
        let items = vec![NullableObjectId(None), ObjectId::new(10).unwrap().into()];
        let render = |reference, index| {
            let ctx = egui::Context::default();
            let out = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    render_list_item(ui, &pool, reference, index, &items)
                });
            });
            out.shapes
                .iter()
                .filter(|shape| matches!(&shape.shape, egui::Shape::Rect(_)))
                .count()
        };
        let blank = render(NullableObjectId(None), 0);
        assert!(render(ObjectId::new(12).unwrap().into(), 0) > blank);
        assert_eq!(render(NullableObjectId(None), 255), blank);
        assert_eq!(render(NullableObjectId(None), 2), blank);
    }

    // Sample the emitted rectangles after applying egui's painter clip, so these
    // assertions exercise previews rather than the scanline implementation.
    fn shape_samples(
        object: &Object,
        pool: &ObjectPool,
        samples: &[(f32, f32)],
    ) -> Vec<Option<Color32>> {
        let ctx = egui::Context::default();
        let mut origin = egui::Pos2::ZERO;
        let output = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                origin = ui.max_rect().min;
                ui.ctx()
                    .data_mut(|data| data.insert_temp(egui::Id::new("vt_pattern_origin"), origin));
                ui.set_clip_rect(egui::Rect::from_min_size(origin, egui::vec2(24.0, 24.0)));
                object.render(ui, pool, Point::default());
            });
        });
        samples
            .iter()
            .map(|(x, y)| {
                let point = origin + egui::vec2(x + 0.5, y + 0.5);
                output
                    .shapes
                    .iter()
                    .skip(1)
                    .filter_map(|shape| match &shape.shape {
                        egui::Shape::Rect(rect)
                            if rect.rect.contains(point) && shape.clip_rect.contains(point) =>
                        {
                            Some(rect.fill)
                        }
                        _ => None,
                    })
                    .last()
            })
            .collect()
    }

    fn shape_pool(width: u8, art: u16) -> ObjectPool {
        let mut pool = ObjectPool::new();
        let Object::LineAttributes(mut line) = crate::default_object(ObjectType::LineAttributes)
        else {
            unreachable!()
        };
        line.id = ObjectId::new(11).unwrap();
        line.line_width = width;
        line.line_art = art;
        line.line_colour = 2;
        pool.add(Object::LineAttributes(line));
        pool
    }

    #[test]
    fn rectangle_fill_modes_and_suppression_preserve_unfilled_border_footprints() {
        let mut pool = shape_pool(2, 0xffff);
        let Object::OutputRectangle(mut rect) = crate::default_object(ObjectType::OutputRectangle)
        else {
            unreachable!()
        };
        rect.width = 20;
        rect.height = 16;
        rect.line_attributes = ObjectId::new(11).unwrap().into();
        rect.fill_attributes = ObjectId::new(12).unwrap().into();
        let Object::FillAttributes(mut fill) = crate::default_object(ObjectType::FillAttributes)
        else {
            unreachable!()
        };
        fill.id = ObjectId::new(12).unwrap();
        fill.fill_colour = 4;
        let line_colour = crate::effective_colour::colour(&pool, 2).convert();
        let fill_colour = crate::effective_colour::colour(&pool, 4).convert();
        for (mode, expected) in [
            (0, None),
            (1, Some(line_colour)),
            (2, Some(fill_colour)),
            (3, None),
        ] {
            fill.fill_type = mode;
            fill.fill_pattern = NullableObjectId(None);
            let mut candidate = pool.clone();
            candidate.add(Object::FillAttributes(fill.clone()));
            assert_eq!(
                shape_samples(
                    &Object::OutputRectangle(rect.clone()),
                    &candidate,
                    &[(10.0, 8.0)]
                ),
                vec![expected]
            );
        }
        fill.fill_type = 2;
        pool.add(Object::FillAttributes(fill.clone()));
        for (suppression, sample) in [
            (1, (10.0, 0.0)),
            (2, (19.0, 8.0)),
            (4, (10.0, 15.0)),
            (8, (0.0, 8.0)),
        ] {
            rect.line_suppression = suppression;
            assert_eq!(
                shape_samples(
                    &Object::OutputRectangle(rect.clone()),
                    &pool,
                    &[sample, (10.0, 8.0)]
                ),
                vec![None, Some(fill_colour)]
            );
        }
        let Object::PictureGraphic(mut picture) = crate::default_object(ObjectType::PictureGraphic)
        else {
            unreachable!()
        };
        picture.id = ObjectId::new(13).unwrap();
        picture.actual_width = 2;
        picture.actual_height = 1;
        picture.width = 200; // Pattern uses raw dimensions, ignoring display scaling.
        picture.format =
            ag_iso_stack::object_pool::object_attributes::PictureGraphicFormat::EightBit;
        picture.data = vec![2, 4];
        picture.options.transparent = true;
        picture.transparency_colour = 2;
        pool.add(Object::PictureGraphic(picture));
        fill.fill_type = 3;
        fill.fill_pattern = ObjectId::new(13).unwrap().into();
        pool.remove(fill.id);
        pool.add(Object::FillAttributes(fill));
        assert_eq!(
            shape_samples(
                &Object::OutputRectangle(rect),
                &pool,
                &[(10.0, 8.0), (11.0, 8.0)]
            ),
            vec![Some(line_colour), Some(fill_colour)]
        );
    }

    #[test]
    fn lines_and_polygons_use_line_art_square_brushes_and_declared_clipping() {
        for width in [1, 2, 4] {
            let pool = shape_pool(width, 0xaaaa);
            let colour = crate::effective_colour::colour(&pool, 2).convert();
            let Object::OutputLine(mut line) = crate::default_object(ObjectType::OutputLine) else {
                unreachable!()
            };
            line.width = 48;
            line.height = u16::from(width);
            line.line_attributes = ObjectId::new(11).unwrap().into();
            let samples: Vec<_> = (0..24).map(|x| (x as f32, f32::from(width - 1))).collect();
            let expected: Vec<_> = (0..24)
                .map(|x| {
                    if (x / width) % 2 == 0 {
                        Some(colour)
                    } else {
                        None
                    }
                })
                .collect();
            assert_eq!(
                shape_samples(&Object::OutputLine(line), &pool, &samples),
                expected
            );
        }
        let pool = shape_pool(2, 0xff00);
        let colour = crate::effective_colour::colour(&pool, 2).convert();
        let Object::OutputLine(mut line) = crate::default_object(ObjectType::OutputLine) else {
            unreachable!()
        };
        line.width = 40;
        line.height = 2;
        line.line_attributes = ObjectId::new(11).unwrap().into();
        assert_eq!(
            shape_samples(
                &Object::OutputLine(line.clone()),
                &pool,
                &[(0.0, 0.0), (10.0, 1.0), (20.0, 0.0), (25.0, 0.0)]
            ),
            vec![Some(colour), Some(colour), None, None]
        );
        let Object::OutputRectangle(mut rectangle) =
            crate::default_object(ObjectType::OutputRectangle)
        else {
            unreachable!()
        };
        rectangle.width = 24;
        rectangle.height = 12;
        rectangle.line_attributes = ObjectId::new(11).unwrap().into();
        assert_eq!(
            shape_samples(
                &Object::OutputRectangle(rectangle),
                &pool,
                &[(4.0, 0.0), (20.0, 0.0)]
            ),
            vec![Some(colour), None]
        );
        let solid_pool = shape_pool(2, 0xffff);
        line.width = 12;
        line.height = 10;
        line.line_direction = LineDirection::BottomLeftToTopRight;
        assert_eq!(
            shape_samples(
                &Object::OutputLine(line),
                &solid_pool,
                &[(0.0, 8.0), (10.0, 0.0), (0.0, 10.0)]
            ),
            vec![Some(colour), Some(colour), None]
        );
        let Object::OutputPolygon(mut polygon) = crate::default_object(ObjectType::OutputPolygon)
        else {
            unreachable!()
        };
        polygon.width = 12;
        polygon.height = 12;
        polygon.line_attributes = ObjectId::new(11).unwrap().into();
        polygon.points = vec![
            Point { x: 0, y: 0 },
            Point { x: 40, y: 0 },
            Point { x: 40, y: 40 },
            Point { x: 0, y: 40 },
        ];
        polygon.polygon_type = 3;
        assert_eq!(
            shape_samples(
                &Object::OutputPolygon(polygon.clone()),
                &pool,
                &[(4.0, 0.0), (14.0, 0.0), (0.0, 6.0)]
            ),
            vec![Some(colour), None, None]
        );
        polygon.width = 24;
        assert_eq!(
            shape_samples(
                &Object::OutputPolygon(polygon.clone()),
                &pool,
                &[(4.0, 0.0), (20.0, 0.0)]
            ),
            vec![Some(colour), None]
        );
        polygon.width = 12;
        let mut filled_pool = solid_pool;
        let Object::FillAttributes(mut fill) = crate::default_object(ObjectType::FillAttributes)
        else {
            unreachable!()
        };
        fill.id = ObjectId::new(12).unwrap();
        fill.fill_type = 1;
        filled_pool.add(Object::FillAttributes(fill));
        polygon.fill_attributes = ObjectId::new(12).unwrap().into();
        assert_eq!(
            shape_samples(
                &Object::OutputPolygon(polygon.clone()),
                &filled_pool,
                &[(6.0, 6.0)]
            ),
            vec![None]
        );
        polygon.polygon_type = 2;
        assert_eq!(
            shape_samples(
                &Object::OutputPolygon(polygon),
                &filled_pool,
                &[(6.0, 6.0), (14.0, 6.0)]
            ),
            vec![Some(colour), None]
        );
        let mut pool = shape_pool(0, 0);
        let Object::FillAttributes(mut fill) = crate::default_object(ObjectType::FillAttributes)
        else {
            unreachable!()
        };
        fill.id = ObjectId::new(12).unwrap();
        fill.fill_type = 2;
        fill.fill_colour = 4;
        let fill_colour = crate::effective_colour::colour(&pool, 4).convert();
        pool.add(Object::FillAttributes(fill));
        let Object::OutputPolygon(mut polygon) = crate::default_object(ObjectType::OutputPolygon)
        else {
            unreachable!()
        };
        polygon.width = 20;
        polygon.height = 20;
        polygon.line_attributes = ObjectId::new(11).unwrap().into();
        polygon.fill_attributes = ObjectId::new(12).unwrap().into();
        polygon.polygon_type = 2;
        polygon.points = vec![
            Point { x: 0, y: 0 },
            Point { x: 20, y: 20 },
            Point { x: 0, y: 20 },
            Point { x: 20, y: 0 },
        ];
        assert_eq!(
            shape_samples(
                &Object::OutputPolygon(polygon),
                &pool,
                &[(10.0, 2.0), (2.0, 10.0), (10.0, 18.0)]
            ),
            vec![Some(fill_colour), None, Some(fill_colour)]
        );
    }

    #[test]
    fn rendered_ellipse_preserves_complete_border_inside_declared_dimensions() {
        for width in [1_u8, 4] {
            let mut pool = ObjectPool::new();
            let mut line = crate::default_object(ObjectType::LineAttributes);
            *line.mut_id() = ObjectId::new(11).unwrap();
            let Object::LineAttributes(attributes) = &mut line else {
                unreachable!()
            };
            attributes.line_width = width;
            attributes.line_art = 0xffff;
            attributes.line_colour = 2;
            let colour = crate::effective_colour::colour(&pool, attributes.line_colour).convert();
            pool.add(line);
            let Object::OutputEllipse(mut ellipse) =
                crate::default_object(ObjectType::OutputEllipse)
            else {
                unreachable!()
            };
            ellipse.width = 100;
            ellipse.height = 60;
            ellipse.ellipse_type = 0;
            ellipse.line_attributes = ObjectId::new(11).unwrap().into();
            ellipse.fill_attributes = NullableObjectId(None);
            let ctx = egui::Context::default();
            let mut expected = egui::Rect::NOTHING;
            let output = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    expected =
                        egui::Rect::from_min_size(ui.max_rect().min, egui::vec2(100.0, 60.0));
                    ellipse.render(ui, &pool, Point::default());
                });
            });
            let painted: Vec<_> = output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect) if rect.fill == colour => Some(rect.rect),
                    _ => None,
                })
                .collect();
            assert!(!painted.is_empty());
            assert!(painted
                .iter()
                .all(|rect| expected.expand(0.001).contains_rect(*rect)));
            let bounds = painted
                .iter()
                .fold(egui::Rect::NOTHING, |bounds, rect| bounds.union(*rect));
            assert!((bounds.top() - expected.top()).abs() < 0.001);
            assert!((bounds.bottom() - expected.bottom()).abs() < 0.001);
            assert!((bounds.left() - expected.left()).abs() < 0.001);
            assert!((bounds.right() - expected.right()).abs() < 0.001);
        }
    }

    #[test]
    fn input_string_renders_variable_contents_and_disabled_indication() {
        let mut pool = ObjectPool::new();
        let mut font = crate::default_object(ObjectType::FontAttributes);
        *font.mut_id() = ObjectId::new(1).unwrap();
        if let Object::FontAttributes(font) = &mut font {
            font.font_size =
                ag_iso_stack::object_pool::object_attributes::FontSize::Proportional(12);
        }
        pool.add(font);
        let mut variable = crate::default_object(ObjectType::StringVariable);
        *variable.mut_id() = ObjectId::new(2).unwrap();
        if let Object::StringVariable(variable) = &mut variable {
            variable.value = "AB".into();
        }
        pool.add(variable);
        let Object::InputString(mut input) = crate::default_object(ObjectType::InputString) else {
            unreachable!()
        };
        input.font_attributes = ObjectId::new(1).unwrap().into();
        input.variable_reference = ObjectId::new(2).unwrap().into();
        input.value = "X".into();
        input.width = 24;
        input.height = 24;
        for enabled in [true, false] {
            input.enabled = enabled;
            let ctx = egui::Context::default();
            let out = ctx.run(Default::default(), |ctx| {
                egui::CentralPanel::default()
                    .show(ctx, |ui| input.render(ui, &pool, Point::default()));
            });
            let glyphs: Vec<_> = out
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Text(text) => Some(text.galley.job.text.as_str()),
                    _ => None,
                })
                .collect();
            assert_eq!(glyphs, ["A", "B"]);
            let overlay=out.shapes.iter().any(|shape| matches!(&shape.shape,
                egui::Shape::Rect(rect) if rect.fill==Color32::from_rgba_premultiplied(128,128,128,100)));
            assert_eq!(overlay, !enabled);
        }
    }

    #[test]
    fn object_layout_preserves_dimensions_at_parent_edges() {
        let ctx = egui::Context::default();
        let _ = ctx.run(Default::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let size = ui.max_rect().size();
                let rect = create_relative_rect(
                    ui,
                    Point {
                        x: (size.x - 5.0) as i16,
                        y: 0,
                    },
                    egui::vec2(100.0, 30.0),
                );
                assert_eq!(rect.width(), 100.0);
                assert_eq!(rect.height(), 30.0);
            });
        });
    }
}
