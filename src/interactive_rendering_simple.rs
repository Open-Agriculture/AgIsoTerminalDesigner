//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

use crate::RenderableObject;
use ag_iso_stack::object_pool::object_attributes::Point;
use ag_iso_stack::object_pool::{object::Object, ObjectId, ObjectPool};
use eframe::egui;

/// Interactive wrapper for rendering masks with clickable objects
pub struct InteractiveMaskRenderer<'a> {
    pub object: &'a Object,
    pub pool: &'a ObjectPool,
    pub selected_callback: Box<dyn FnMut(ObjectId) + 'a>,
}

impl<'a> egui::Widget for InteractiveMaskRenderer<'a> {
    fn ui(mut self, ui: &mut egui::Ui) -> egui::Response {
        // Create an interactive area for the entire mask
        let (width, height) = self.pool.content_size(self.object);
        let desired_size = egui::vec2(width as f32, height as f32);
        let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click());

        if ui.is_rect_visible(rect) {
            // Create a child UI for rendering the objects
            let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(rect));

            // Render the objects normally
            self.object
                .render(&mut child_ui, self.pool, Point::default());

            // Handle interaction - check if pointer is interacting with this widget
            if let Some(pointer_pos) = ui.ctx().pointer_hover_pos() {
                // Check if the pointer is within our allocated rect
                if rect.contains(pointer_pos) {
                    // Convert screen position to widget-relative position
                    let relative_pos =
                        egui::pos2(pointer_pos.x - rect.min.x, pointer_pos.y - rect.min.y);

                    // Find what object is under the hover position
                    if let Some((object_id, object_rect)) = self.find_object_at(relative_pos) {
                        // Draw highlight rectangle around the object
                        let screen_rect = egui::Rect::from_min_size(
                            rect.min + object_rect.min.to_vec2(),
                            object_rect.size(),
                        );
                        ui.painter().rect_stroke(
                            screen_rect,
                            0.0,
                            egui::Stroke::new(
                                2.0,
                                egui::Color32::from_rgba_premultiplied(255, 255, 0, 200),
                            ),
                            egui::epaint::StrokeKind::Middle,
                        );

                        // Draw circle at pointer position
                        ui.painter().circle_stroke(
                            pointer_pos,
                            10.0,
                            egui::Stroke::new(
                                2.0,
                                egui::Color32::from_rgba_premultiplied(255, 255, 255, 128),
                            ),
                        );

                        if response.clicked() {
                            (self.selected_callback)(object_id);
                            ui.ctx().request_repaint(); // Force UI update
                        }
                    }
                }
            }
        }

        response
    }
}

impl<'a> InteractiveMaskRenderer<'a> {
    /// Find which object is at the given position (relative to widget)
    fn find_object_at(&self, pos: egui::Pos2) -> Option<(ObjectId, egui::Rect)> {
        self.find_object_recursive(self.object, Point::default(), pos)
    }

    fn find_object_recursive(
        &self,
        object: &Object,
        offset: Point<i16>,
        pos: egui::Pos2,
    ) -> Option<(ObjectId, egui::Rect)> {
        let (width, height) = self.pool.content_size(object);
        let rect = egui::Rect::from_min_size(
            egui::pos2(offset.x as f32, offset.y as f32),
            egui::vec2(width as f32, height as f32),
        );

        // Check children first (they're on top)
        match object {
            Object::DataMask(mask) => {
                for obj_ref in mask.object_refs.iter().rev() {
                    if let Some(child) = self.pool.object_by_id(obj_ref.id) {
                        let child_offset = Point {
                            x: offset.x + obj_ref.offset.x,
                            y: offset.y + obj_ref.offset.y,
                        };
                        if let Some(result) = self.find_object_recursive(child, child_offset, pos) {
                            return Some(result);
                        }
                    }
                }
            }
            Object::AlarmMask(mask) => {
                for obj_ref in mask.object_refs.iter().rev() {
                    if let Some(child) = self.pool.object_by_id(obj_ref.id) {
                        let child_offset = Point {
                            x: offset.x + obj_ref.offset.x,
                            y: offset.y + obj_ref.offset.y,
                        };
                        if let Some(result) = self.find_object_recursive(child, child_offset, pos) {
                            return Some(result);
                        }
                    }
                }
            }
            Object::Container(container) => {
                for obj_ref in container.object_refs.iter().rev() {
                    if let Some(child) = self.pool.object_by_id(obj_ref.id) {
                        let child_offset = Point {
                            x: offset.x + obj_ref.offset.x,
                            y: offset.y + obj_ref.offset.y,
                        };
                        if let Some(result) = self.find_object_recursive(child, child_offset, pos) {
                            return Some(result);
                        }
                    }
                }
            }
            _ => {}
        }

        // Then check this object
        if rect.contains(pos) {
            Some((object.id(), rect))
        } else {
            None
        }
    }
}
