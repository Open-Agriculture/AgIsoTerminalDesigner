//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

use std::collections::hash_map::DefaultHasher;
use std::hash::Hash;
use std::hash::Hasher;

use ag_iso_stack::object_pool::object::*;
use ag_iso_stack::object_pool::object_attributes::PictureGraphicFormat;
use ag_iso_stack::object_pool::object_attributes::Point;
use ag_iso_stack::object_pool::Colour;
use ag_iso_stack::object_pool::ObjectPool;
use ag_iso_stack::object_pool::ObjectRef;
use eframe::egui;
use eframe::egui::Color32;
use eframe::egui::ColorImage;
use eframe::egui::TextureHandle;
use eframe::egui::TextureId;

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

        ui.allocate_ui_at_rect(rect, |ui| {
            render_object_refs(ui, pool, &self.object_refs);
        });
    }
}

impl RenderableObject for Button {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        const BUTTON_BORDER_WIDTH: f32 = 4.0;

        let rect = create_relative_rect(
            ui,
            position,
            egui::Vec2::new(self.width as f32, self.height as f32),
        );

        let button_face = if self.options.no_border {
            rect
        } else {
            rect.shrink(BUTTON_BORDER_WIDTH)
        };

        let mut button = egui::Button::new("").fill(egui::Color32::TRANSPARENT);
        if !self.options.transparent_background {
            button = button.fill(pool.color_by_index(self.background_colour).convert());
        };

        ui.visuals_mut().selection.stroke.width = if self.options.suppress_border {
            0.0
        } else {
            BUTTON_BORDER_WIDTH
        };
        ui.visuals_mut().widgets.hovered.bg_stroke.width = if self.options.suppress_border {
            0.0
        } else {
            BUTTON_BORDER_WIDTH
        };

        ui.put(button_face, button);

        ui.allocate_ui_at_rect(button_face, |ui| {
            render_object_refs(ui, pool, &self.object_refs);
        });
    }
}

impl RenderableObject for Key {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let rect = create_relative_rect(ui, position, egui::Vec2::new(100.0, 100.0));

        ui.allocate_ui_at_rect(rect, |ui| {
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

        let is_transparent = self.options.transparent;
        let is_auto_wrap = self.options.auto_wrap;
        let is_wrap_on_hyphen = self.options.wrap_on_hyphen;
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
        let text = if let Some(variable_reference_id) = self.variable_reference.into() {
            match pool.object_by_id(variable_reference_id) {
                Some(Object::StringVariable(s)) => s.value.clone(),
                _ => self.value.clone(),
            }
        } else {
            self.value.clone()
        };
        let horizontal_justification = self.justification.horizontal;
        let vertical_justification = self.justification.vertical;

        // TODO: Implement text wrap on hyphen
        // TODO: Implement text justification
        // TODO: implement text size

        ui.allocate_ui_at_rect(rect, |ui| {
            ui.colored_label(
                pool.color_by_index(font_attributes.font_colour).convert(),
                text,
            );
        });
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

        ui.allocate_ui_at_rect(rect, |ui| {
            if let Some(texture_id) = texture_id {
                ui.image((texture_id, rect.size()));
            } else {
                ui.label("Failed to load image");
            }
        });
    }
}
