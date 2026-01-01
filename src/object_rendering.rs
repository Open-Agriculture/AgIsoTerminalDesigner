//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

use std::collections::hash_map::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;
use std::ops::Sub;

use ag_iso_stack::object_pool::object::*;
use ag_iso_stack::object_pool::object_attributes::ButtonState;
use ag_iso_stack::object_pool::object_attributes::FontSize;
use ag_iso_stack::object_pool::object_attributes::FormatType;
use ag_iso_stack::object_pool::object_attributes::HorizontalAlignment;
use ag_iso_stack::object_pool::object_attributes::LineDirection;
use ag_iso_stack::object_pool::object_attributes::PictureGraphicFormat;
use ag_iso_stack::object_pool::object_attributes::Point;
use ag_iso_stack::object_pool::object_attributes::VerticalAlignment;
use ag_iso_stack::object_pool::vt_version::VtVersion;
use ag_iso_stack::object_pool::Colour;
use ag_iso_stack::object_pool::ObjectPool;
use ag_iso_stack::object_pool::ObjectRef;
use eframe::egui;
use eframe::egui::Color32;
use eframe::egui::ColorImage;
use eframe::egui::FontId;
use eframe::egui::TextWrapMode;
use eframe::egui::TextureHandle;
use eframe::egui::TextureId;
use eframe::egui::UiBuilder;

pub trait RenderableObject {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>);
}

impl RenderableObject for Object {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        // Make sure text is truncated if it doesn't fit for all object renderings (useful for error labels)
        ui.style_mut().wrap_mode = Some(TextWrapMode::Truncate);

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
            Object::AuxiliaryFunctionType1(o) => (),
            Object::AuxiliaryInputType1(o) => (),
            Object::AuxiliaryFunctionType2(o) => o.render(ui, pool, position),
            Object::AuxiliaryInputType2(o) => o.render(ui, pool, position),
            Object::AuxiliaryControlDesignatorType2(o) => o.render(ui, pool, position),
            Object::WindowMask(o) => (),
            Object::KeyGroup(o) => (),
            Object::GraphicsContext(o) => (),
            Object::ExtendedInputAttributes(o) => (),
            Object::ColourMap(o) => (),
            Object::ObjectLabelReferenceList(o) => (),
            Object::ExternalObjectDefinition(o) => (),
            Object::ExternalReferenceName(o) => (),
            Object::ExternalObjectPointer(o) => (),
            Object::Animation(o) => (),
            Object::ColourPalette(o) => (),
            Object::GraphicData(o) => (),
            Object::WorkingSetSpecialControls(o) => (),
            Object::ScaledGraphic(o) => (),
        }
    }
}

trait Colorable {
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

impl RenderableObject for WorkingSet {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, _: Point<i16>) {
        ui.painter().rect_filled(
            ui.available_rect_before_wrap(),
            0.0,
            pool.color_by_index(self.background_colour).convert(),
        );

        render_object_refs(ui, pool, &self.object_refs);
    }
}

impl RenderableObject for DataMask {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, _: Point<i16>) {
        ui.painter().rect_filled(
            ui.available_rect_before_wrap(),
            0.0,
            pool.color_by_index(self.background_colour).convert(),
        );

        render_object_refs(ui, pool, &self.object_refs);
    }
}

impl RenderableObject for AlarmMask {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, _: Point<i16>) {
        ui.painter().rect_filled(
            ui.available_rect_before_wrap(),
            0.0,
            pool.color_by_index(self.background_colour).convert(),
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
            render_object_refs(ui, pool, &self.object_refs);
        });
    }
}

impl RenderableObject for Button {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let vt_version = VtVersion::Version3;

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
        const BORDER_WIDTH: f32 = 4.0;
        let face_rect = if no_border {
            rect
        } else {
            // Face is area minus 8 pixels in width and height.
            // We'll just evenly shrink by 4 pixels on each side.
            rect.shrink(BORDER_WIDTH)
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
            let color = pool.color_by_index(self.background_colour).convert();
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
            let color = pool.color_by_index(self.border_colour).convert();
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
                egui::Stroke::new(BORDER_WIDTH, border_color),
                egui::StrokeKind::Inside,
            );
        }

        ui.painter().rect_filled(face_rect, 0.0, background_color);

        // Child objects are clipped to the face area
        ui.scope_builder(UiBuilder::new().max_rect(face_rect), |ui| {
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
        let is_true = if let Some(var_id) = self.variable_reference.0 {
            match pool.object_by_id(var_id) {
                Some(Object::NumberVariable(num_var)) => num_var.value > 0,
                _ => self.value,
            }
        } else {
            self.value
        };

        let side = self.width as f32;
        let rect = create_relative_rect(ui, position, egui::Vec2::new(side, side));

        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            let background_color = pool.color_by_index(self.background_colour).convert();
            ui.painter().rect_filled(rect, 0.0, background_color);

            // If the boolean is true, we display a checkmark in the center
            if is_true {
                let fg_color = match pool.object_by_id(self.foreground_colour) {
                    Some(Object::FontAttributes(font_attr)) => {
                        pool.color_by_index(font_attr.font_colour).convert()
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
            if !self.enabled {
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
            let font_attributes = match pool.object_by_id(self.font_attributes) {
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
            let background_colour = pool.color_by_index(self.background_colour).convert();
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
                        pool.color_by_index(font_attributes.font_colour).convert(),
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
            let font_colour = pool.color_by_index(font_attributes.font_colour).convert();

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
        let rect = create_relative_rect(ui, position, egui::Vec2::INFINITY); // TODO: set size to softkey area

        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
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
        let rect = create_relative_rect(
            ui,
            position,
            egui::Vec2::new(self.width() as f32, self.height() as f32),
        );

        let font_attributes = match pool.object_by_id(self.font_attributes) {
            Some(Object::FontAttributes(f)) => f,
            _ => {
                ui.colored_label(
                    Color32::RED,
                    format!("Missing font attributes: {:?}", self.font_attributes),
                );
                return;
            }
        };
        let background_colour = pool.color_by_index(self.background_colour).convert();

        let transparent = self.options.transparent;
        let auto_wrap = self.options.auto_wrap;

        // TODO: check if VT version is 4 or later, if so implement wrap_on_hyphen
        // let wrap_on_hyphen = self.options.wrap_on_hyphen;
        // Note: wrap_on_hyphen behavior is complex. For simplicity here, we rely on normal word-wrapping
        // from egui and do not implement special hyphenation logic. A more thorough implementation
        // would detect hyphens and possibly treat them as break opportunities.

        // According to the specification, we need to handle control characters (CR, LF) as line breaks.
        // We'll normalize all line endings to '\n'.
        let mut text_value = if let Some(variable_reference_id) = self.variable_reference.into() {
            match pool.object_by_id(variable_reference_id) {
                Some(Object::StringVariable(s)) => s.value.clone(),
                _ => self.value.clone(),
            }
        } else {
            self.value.clone()
        };
        text_value = text_value
            .replace("\r\n", "\n")
            .replace("\n\r", "\n")
            .replace('\r', "\n")
            .replace('\x0a', "\n");

        // Apply space trimming rules based on horizontal justification:
        // - Left justification: no trimming of leading spaces (for the first line), trailing spaces remain as is.
        // - Middle justification: remove leading and trailing spaces on each line.
        // - Right justification: remove trailing spaces on each line.
        let mut lines: Vec<&str> = text_value.split('\n').collect();
        for (line_number, line) in lines.iter_mut().enumerate() {
            match self.justification.horizontal {
                HorizontalAlignment::Left => {
                    // Per ISO rules, if auto-wrapping is enabled, leading spaces on wrapped lines might be removed.
                    if auto_wrap && line_number > 0 {
                        // Remove leading spaces
                        *line = line.trim_start();
                    }
                }
                HorizontalAlignment::Middle => {
                    // Remove both leading and trailing spaces
                    *line = line.trim();
                }
                HorizontalAlignment::Right => {
                    // Remove trailing spaces only
                    *line = line.trim_end();
                }
                HorizontalAlignment::Reserved => {
                    ui.colored_label(
                        Color32::RED,
                        "Configuration incorrect: horizontal alignment is set to Reserved",
                    );
                    return;
                }
            }
        }

        let processed_text = lines.join("\n");

        let font_colour = pool.color_by_index(font_attributes.font_colour).convert();
        let fonts = ui.fonts(|fonts| fonts.clone());
        let font_height;
        let font_family;
        match font_attributes.font_size {
            FontSize::NonProportional(size) => {
                font_family = egui::FontFamily::Monospace;

                // We need to calculate the font height based on the width of a letter in the monospace font.
                let font_size = fonts
                    .layout_no_wrap(
                        "a".into(),
                        FontId::new(size.height() as f32, egui::FontFamily::Monospace),
                        font_colour,
                    )
                    .size();

                font_height = size.height() as f32 * (font_size.x / size.width() as f32);
            }
            FontSize::Proportional(height) => {
                font_height = height as f32;
                font_family = egui::FontFamily::Proportional;
            }
        }

        let wrap_width = if auto_wrap {
            self.width() as f32
        } else {
            f32::INFINITY
        };

        let galley = fonts.layout(
            processed_text,
            FontId::new(font_height, font_family.clone()),
            font_colour,
            wrap_width,
        );

        let text_size = galley.size();

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
        };

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
        };

        if !transparent {
            let painter = ui.painter();
            painter.rect_filled(rect, 0.0, background_colour);
        }

        ui.painter().galley(paint_pos, galley, font_colour);
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
            let font_attributes = match pool.object_by_id(self.font_attributes) {
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
            let background_colour = pool.color_by_index(self.background_colour).convert();
            let font_colour = pool.color_by_index(font_attributes.font_colour).convert();

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
            ui.colored_label(Color32::RED, "OutputList not implemented");
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
            let line_attributes = match pool.object_by_id(self.line_attributes) {
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

            let colour = pool.color_by_index(line_attributes.line_colour).convert();
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
        let line_attributes = match pool.object_by_id(self.line_attributes) {
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
                pool.color_by_index(fill_attributes.fill_colour).convert(),
            );
            // TODO: implement fill type for infill
            // TODO: implement fill pattern for infill
        }

        ui.painter().rect_stroke(
            rect,
            0.0,
            egui::Stroke::new(
                line_attributes.line_width,
                pool.color_by_index(line_attributes.line_colour).convert(),
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
            ui.colored_label(Color32::RED, "OutputPolygon not implemented");
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

        let mut hasher = DefaultHasher::new();
        Object::PictureGraphic(self.clone())
            .write()
            .hash(&mut hasher);
        let hash = hasher.finish();

        let changed: bool = ui.data_mut(|data| {
            let old_hash: Option<u64> =
                data.get_temp(format!("picturegraphic_{}_image", self.id.value()).into());
            if old_hash.is_none() || old_hash.unwrap() != hash {
                data.insert_temp(
                    format!("picturegraphic_{}_image", self.id.value()).into(),
                    hash,
                );
                true
            } else {
                false
            }
        });

        let texture_id: Option<TextureId>;
        if changed {
            let mut x = 0;
            let mut y = 0;

            let mut image = ColorImage::filled(
                [self.actual_width.into(), self.actual_height.into()],
                Color32::TRANSPARENT,
            );

            for raw in self.data_as_raw_encoded() {
                let mut colors: Vec<Color32> = vec![];
                match self.format {
                    PictureGraphicFormat::Monochrome => {
                        for bit in 0..8 {
                            colors.push(pool.color_by_index((raw >> (7 - bit)) & 0x01).convert());
                        }
                    }
                    PictureGraphicFormat::FourBit => {
                        for segment in 0..2 {
                            let shift = 4 - (segment * 4);
                            colors.push(pool.color_by_index((raw >> shift) & 0x0F).convert());
                        }
                    }
                    PictureGraphicFormat::EightBit => {
                        colors.push(pool.color_by_index(raw).convert());
                    }
                }

                for color in colors {
                    let idx = y as usize * self.actual_width as usize + x as usize;
                    if idx >= image.pixels.len() {
                        break;
                    }
                    if !(self.options.transparent
                        && color == pool.color_by_index(self.transparency_colour).convert())
                    {
                        image.pixels[idx] = color;
                    }

                    x += 1;
                    if x >= self.actual_width {
                        x = 0;
                        y += 1;
                        // If we go onto the next row, then we discard the rest of the bits
                        break;
                    }
                }
            }

            let new_texture = ui.ctx().load_texture(
                format!("picturegraphic_{}_texture", self.id.value()).as_str(),
                image,
                Default::default(),
            );
            texture_id = Some(new_texture.id());
            ui.data_mut(|data| {
                println!("Saving texture - {:?}", self.id.value());
                data.insert_temp(
                    format!("picturegraphic_{}_texture", self.id.value()).into(),
                    new_texture,
                );
            });
        } else {
            texture_id = ui.data(|data| {
                data.get_temp::<TextureHandle>(
                    format!("picturegraphic_{}_texture", self.id.value()).into(),
                )
                .map(|t| t.id())
            });
        }

        // Use image dimensions, but clip to the available rect
        let image_size = egui::Vec2::new(self.width as f32, self.height() as f32);
        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            if let Some(texture_id) = texture_id {
                ui.image((texture_id, image_size));
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
            let background_colour = pool.color_by_index(self.background_colour).convert();
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
            let background_colour = pool.color_by_index(self.background_colour).convert();
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
