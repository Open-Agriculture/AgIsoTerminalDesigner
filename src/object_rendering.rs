//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

use std::collections::hash_map::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;

use ag_iso_stack::object_pool::object::*;
use ag_iso_stack::object_pool::object_attributes::ButtonState;
use ag_iso_stack::object_pool::object_attributes::FontSize;
use ag_iso_stack::object_pool::object_attributes::HorizontalAlignment;
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
use eframe::egui::TextureHandle;
use eframe::egui::TextureId;
use eframe::egui::UiBuilder;

pub trait RenderableObject {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>);
}

impl RenderableObject for Object {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        match self {
            Object::WorkingSet(o) => o.render(ui, pool, position),
            Object::DataMask(o) => o.render(ui, pool, position),
            Object::AlarmMask(o) => o.render(ui, pool, position),
            Object::Container(o) => o.render(ui, pool, position),
            Object::SoftKeyMask(o) => (),
            Object::Key(o) => o.render(ui, pool, position),
            Object::Button(o) => o.render(ui, pool, position),
            Object::InputBoolean(o) => (),
            Object::InputString(o) => (),
            Object::InputNumber(o) => (),
            Object::InputList(o) => (),
            Object::OutputString(o) => o.render(ui, pool, position),
            Object::OutputNumber(o) => (),
            Object::OutputList(o) => (),
            Object::OutputLine(o) => (),
            Object::OutputRectangle(o) => o.render(ui, pool, position),
            Object::OutputEllipse(o) => (),
            Object::OutputPolygon(o) => (),
            Object::OutputMeter(o) => (),
            Object::OutputLinearBarGraph(o) => (),
            Object::OutputArchedBarGraph(o) => (),
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
            Object::AuxiliaryFunctionType2(o) => (),
            Object::AuxiliaryInputType2(o) => (),
            Object::AuxiliaryControlDesignatorType2(o) => (),
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
    egui::Rect::from_min_size(
        ui.max_rect().min + egui::Vec2::new(position.x as f32, position.y as f32),
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
                ui.label(format!("Missing object: {:?}", object));
            }
        }
    }
}

impl RenderableObject for WorkingSet {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, _: Point<i16>) {
        if !self.selectable {
            // The working set is not visible
            return;
        }

        egui::Frame::none()
            .fill(pool.color_by_index(self.background_colour).convert())
            .show(ui, |ui| {
                render_object_refs(ui, pool, &self.object_refs);
            });
    }
}

impl RenderableObject for DataMask {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, _: Point<i16>) {
        egui::Frame::none()
            .fill(pool.color_by_index(self.background_colour).convert())
            .show(ui, |ui| {
                render_object_refs(ui, pool, &self.object_refs);
            });
    }
}

impl RenderableObject for AlarmMask {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, _: Point<i16>) {
        egui::Frame::none()
            .fill(pool.color_by_index(self.background_colour).convert())
            .show(ui, |ui| {
                render_object_refs(ui, pool, &self.object_refs);
            });
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
            egui::Vec2::new(self.width as f32, self.height as f32),
        );

        ui.allocate_new_ui(UiBuilder::new().max_rect(rect), |ui| {
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
            egui::Vec2::new(self.width as f32, self.height as f32),
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
                face_rect,
                0.0,
                egui::Stroke::new(BORDER_WIDTH, border_color),
            );
        }

        ui.painter().rect_filled(face_rect, 0.0, background_color);

        // Child objects are clipped to the face area
        ui.allocate_new_ui(UiBuilder::new().max_rect(face_rect), |ui| {
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

impl RenderableObject for Key {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let rect = create_relative_rect(ui, position, egui::Vec2::new(100.0, 100.0));

        ui.allocate_new_ui(UiBuilder::new().max_rect(rect), |ui| {
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
                ui.label(format!("Missing object: {:?}", self));
            }
        }
    }
}

impl RenderableObject for OutputString {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let rect = create_relative_rect(
            ui,
            position,
            egui::Vec2::new(self.width as f32, self.height as f32),
        );

        let font_attributes = match pool.object_by_id(self.font_attributes) {
            Some(Object::FontAttributes(f)) => f,
            _ => {
                ui.label(format!(
                    "Missing font attributes: {:?}",
                    self.font_attributes
                ));
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
            self.width as f32
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

impl RenderableObject for OutputRectangle {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let rect = create_relative_rect(
            ui,
            position,
            egui::Vec2::new(self.width as f32, self.height as f32),
        );

        // Paint the border of the rectangle
        let line_attributes = match pool.object_by_id(self.line_attributes) {
            Some(Object::LineAttributes(l)) => l,
            _ => {
                ui.label(format!(
                    "Missing line attributes: {:?}",
                    self.line_attributes
                ));
                return;
            }
        };
        ui.painter().rect_stroke(
            rect,
            0.0,
            egui::Stroke::new(
                line_attributes.line_width,
                pool.color_by_index(line_attributes.line_colour).convert(),
            ),
        );
        // TODO: implement line art for border

        // Paint the fill of the rectangle
        if let Some(fill) = self.fill_attributes.into() {
            let fill_attributes = match pool.object_by_id(fill) {
                Some(Object::FillAttributes(f)) => f,
                _ => {
                    ui.label(format!("Missing fill attributes: {:?}", fill));
                    return;
                }
            };
            ui.painter().rect_filled(
                rect.shrink(line_attributes.line_width as f32),
                0.0,
                pool.color_by_index(fill_attributes.fill_colour).convert(),
            );
            // TODO: implement fill type for infill
            // TODO: implement fill pattern for infill
        }
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

            let mut image = ColorImage::new(
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

        ui.allocate_new_ui(UiBuilder::new().max_rect(rect), |ui| {
            if let Some(texture_id) = texture_id {
                ui.image((texture_id, rect.size()));
            } else {
                ui.label("Failed to load image");
            }
        });
    }
}
