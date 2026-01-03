//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

use crate::RenderableObject;
use ag_iso_stack::object_pool::object_attributes::Point;
use ag_iso_stack::object_pool::{object::Object, ObjectId, ObjectPool};
use eframe::egui;

/// Handle position for resizing
#[derive(Debug, Clone, Copy, PartialEq)]
enum ResizeHandle {
    TopLeft,
    Top,
    TopRight,
    Right,
    BottomRight,
    Bottom,
    BottomLeft,
    Left,
}

impl ResizeHandle {
    fn cursor(&self) -> egui::CursorIcon {
        match self {
            ResizeHandle::TopLeft | ResizeHandle::BottomRight => egui::CursorIcon::ResizeNwSe,
            ResizeHandle::Top | ResizeHandle::Bottom => egui::CursorIcon::ResizeVertical,
            ResizeHandle::TopRight | ResizeHandle::BottomLeft => egui::CursorIcon::ResizeNeSw,
            ResizeHandle::Right | ResizeHandle::Left => egui::CursorIcon::ResizeHorizontal,
        }
    }
}

/// Drag state stored in egui memory
#[derive(Clone, Debug)]
struct DragState {
    object_id: ObjectId,
    start_pos: egui::Pos2,
    is_resizing: Option<ResizeHandle>,
}

/// Pending updates to apply to objects (stored in egui context)
#[derive(Clone, Debug, Default)]
pub struct PendingObjectUpdates {
    pub position_updates: Vec<(ObjectId, i16, i16)>,  // (id, delta_x, delta_y)
    pub size_updates: Vec<(ObjectId, i16, i16)>,      // (id, delta_w, delta_h)
}

impl PendingObjectUpdates {
    pub fn add_position_update(&mut self, object_id: ObjectId, delta_x: i16, delta_y: i16) {
        self.position_updates.push((object_id, delta_x, delta_y));
    }

    pub fn add_size_update(&mut self, object_id: ObjectId, delta_w: i16, delta_h: i16) {
        self.size_updates.push((object_id, delta_w, delta_h));
    }

    pub fn clear(&mut self) {
        self.position_updates.clear();
        self.size_updates.clear();
    }
}

/// Interactive wrapper for rendering masks with clickable objects
pub struct InteractiveMaskRenderer<'a> {
    pub object: &'a Object,
    pub pool: &'a ObjectPool,
    pub selected_callback: Box<dyn FnMut(ObjectId) + 'a>,
    pub selected_object_id: Option<ObjectId>,
    pub position_callback: Option<Box<dyn FnMut(ObjectId, i16, i16) + 'a>>,
    pub resize_callback: Option<Box<dyn FnMut(ObjectId, i16, i16) + 'a>>,
}

impl<'a> egui::Widget for InteractiveMaskRenderer<'a> {
    fn ui(mut self, ui: &mut egui::Ui) -> egui::Response {
        // Create an interactive area for the entire mask
        let (width, height) = self.pool.content_size(self.object);
        let desired_size = egui::vec2(width as f32, height as f32);
        let (rect, response) = ui.allocate_exact_size(desired_size, egui::Sense::click_and_drag());

        // Load drag state from memory
        let drag_state_id = ui.id().with("drag_state");
        let mut drag_state: Option<DragState> = ui.ctx().data_mut(|d| d.get_temp(drag_state_id));

        // Load pending updates from context
        let updates_id = egui::Id::new("pending_object_updates");
        let mut pending_updates: PendingObjectUpdates =
            ui.ctx().data_mut(|d| d.get_temp(updates_id).unwrap_or_default());

        if ui.is_rect_visible(rect) {
            // Create a child UI for rendering the objects
            let mut child_ui = ui.new_child(egui::UiBuilder::new().max_rect(rect));

            // Render the objects normally
            self.object
                .render(&mut child_ui, self.pool, Point::default());

            // Handle dragging
            if let Some(ref state) = drag_state {
                if ui.input(|i| i.pointer.primary_down()) {
                    if let Some(pointer_pos) = ui.ctx().pointer_latest_pos() {
                        let delta = pointer_pos - state.start_pos;

                        if let Some(resize_handle) = state.is_resizing {
                            // Queue resize update
                            let (delta_w, delta_h) = match resize_handle {
                                ResizeHandle::Right => (delta.x as i16, 0),
                                ResizeHandle::Bottom => (0, delta.y as i16),
                                ResizeHandle::BottomRight => (delta.x as i16, delta.y as i16),
                                ResizeHandle::Left => (-delta.x as i16, 0),
                                ResizeHandle::Top => (0, -delta.y as i16),
                                ResizeHandle::TopLeft => (-delta.x as i16, -delta.y as i16),
                                ResizeHandle::TopRight => (delta.x as i16, -delta.y as i16),
                                ResizeHandle::BottomLeft => (-delta.x as i16, delta.y as i16),
                            };
                            pending_updates.add_size_update(state.object_id, delta_w, delta_h);
                        } else {
                            // Queue position update
                            pending_updates.add_position_update(state.object_id, delta.x as i16, delta.y as i16);
                        }

                        // Update start position for next frame
                        drag_state = Some(DragState {
                            start_pos: pointer_pos,
                            ..state.clone()
                        });
                    }
                } else {
                    // Mouse released, clear drag state
                    drag_state = None;
                }
            }

            // Handle interaction - check if pointer is interacting with this widget
            if let Some(pointer_pos) = ui.ctx().pointer_hover_pos() {
                // Check if the pointer is within our allocated rect
                if rect.contains(pointer_pos) {
                    // Convert screen position to widget-relative position
                    let relative_pos =
                        egui::pos2(pointer_pos.x - rect.min.x, pointer_pos.y - rect.min.y);

                    // Find what object is under the hover position
                    if let Some((object_id, object_rect)) = self.find_object_at(relative_pos) {
                        let screen_rect = egui::Rect::from_min_size(
                            rect.min + object_rect.min.to_vec2(),
                            object_rect.size(),
                        );

                        // Check if this is the selected object
                        let is_selected = self.selected_object_id == Some(object_id);

                        if is_selected {
                            // Check if hovering over a resize handle
                            let resize_handle = self.get_resize_handle_at(screen_rect, pointer_pos);

                            if let Some(handle) = resize_handle {
                                ui.ctx().set_cursor_icon(handle.cursor());

                                // Start dragging on resize handle
                                if ui.input(|i| i.pointer.primary_pressed()) {
                                    drag_state = Some(DragState {
                                        object_id,
                                        start_pos: pointer_pos,
                                        is_resizing: Some(handle),
                                    });
                                }
                            } else {
                                // Start dragging the object itself
                                if ui.input(|i| i.pointer.primary_pressed()) {
                                    drag_state = Some(DragState {
                                        object_id,
                                        start_pos: pointer_pos,
                                        is_resizing: None,
                                    });
                                }
                            }

                            // Draw selection rectangle with handles
                            ui.painter().rect_stroke(
                                screen_rect,
                                0.0,
                                egui::Stroke::new(
                                    2.0,
                                    egui::Color32::from_rgba_premultiplied(0, 150, 255, 255),
                                ),
                                egui::epaint::StrokeKind::Middle,
                            );

                            // Draw resize handles
                            self.draw_resize_handles(ui, screen_rect);
                        } else {
                            // Draw hover highlight rectangle
                            ui.painter().rect_stroke(
                                screen_rect,
                                0.0,
                                egui::Stroke::new(
                                    2.0,
                                    egui::Color32::from_rgba_premultiplied(255, 255, 0, 200),
                                ),
                                egui::epaint::StrokeKind::Middle,
                            );
                        }

                        if response.clicked() {
                            (self.selected_callback)(object_id);
                            ui.ctx().request_repaint(); // Force UI update
                        }
                    }
                }
            }

            // Draw selection handles even when not hovering if object is selected
            if let Some(selected_id) = self.selected_object_id {
                if let Some((_id, object_rect)) = self.find_object_by_id(selected_id, Point::default()) {
                    let screen_rect = egui::Rect::from_min_size(
                        rect.min + object_rect.min.to_vec2(),
                        object_rect.size(),
                    );

                    // Draw selection rectangle
                    ui.painter().rect_stroke(
                        screen_rect,
                        0.0,
                        egui::Stroke::new(
                            2.0,
                            egui::Color32::from_rgba_premultiplied(0, 150, 255, 255),
                        ),
                        egui::epaint::StrokeKind::Middle,
                    );

                    // Draw resize handles
                    self.draw_resize_handles(ui, screen_rect);
                }
            }
        }

        // Save drag state and pending updates
        ui.ctx().data_mut(|d| {
            d.insert_temp(drag_state_id, drag_state);
            d.insert_temp(updates_id, pending_updates);
        });

        response
    }
}

impl<'a> InteractiveMaskRenderer<'a> {
    /// Check if a position is over a resize handle, return which handle
    fn get_resize_handle_at(&self, rect: egui::Rect, pos: egui::Pos2) -> Option<ResizeHandle> {
        const HANDLE_SIZE: f32 = 8.0;
        const HANDLE_HALF: f32 = HANDLE_SIZE / 2.0;

        let handles = [
            (ResizeHandle::TopLeft, rect.left_top()),
            (ResizeHandle::Top, egui::pos2(rect.center().x, rect.top())),
            (ResizeHandle::TopRight, rect.right_top()),
            (ResizeHandle::Right, egui::pos2(rect.right(), rect.center().y)),
            (ResizeHandle::BottomRight, rect.right_bottom()),
            (ResizeHandle::Bottom, egui::pos2(rect.center().x, rect.bottom())),
            (ResizeHandle::BottomLeft, rect.left_bottom()),
            (ResizeHandle::Left, egui::pos2(rect.left(), rect.center().y)),
        ];

        for (handle_type, handle_pos) in handles.iter() {
            let handle_rect = egui::Rect::from_center_size(*handle_pos, egui::vec2(HANDLE_SIZE, HANDLE_SIZE));
            if handle_rect.expand(HANDLE_HALF).contains(pos) {
                return Some(*handle_type);
            }
        }

        None
    }

    /// Draw resize handles at the corners and edges of a rectangle
    fn draw_resize_handles(&self, ui: &egui::Ui, rect: egui::Rect) {
        const HANDLE_SIZE: f32 = 8.0;
        let handle_color = egui::Color32::from_rgb(0, 150, 255);

        let handles = [
            (ResizeHandle::TopLeft, rect.left_top()),
            (ResizeHandle::Top, egui::pos2(rect.center().x, rect.top())),
            (ResizeHandle::TopRight, rect.right_top()),
            (ResizeHandle::Right, egui::pos2(rect.right(), rect.center().y)),
            (ResizeHandle::BottomRight, rect.right_bottom()),
            (ResizeHandle::Bottom, egui::pos2(rect.center().x, rect.bottom())),
            (ResizeHandle::BottomLeft, rect.left_bottom()),
            (ResizeHandle::Left, egui::pos2(rect.left(), rect.center().y)),
        ];

        for (_handle_type, pos) in handles.iter() {
            let handle_rect = egui::Rect::from_center_size(*pos, egui::vec2(HANDLE_SIZE, HANDLE_SIZE));
            ui.painter().rect_filled(handle_rect, 1.0, handle_color);
            ui.painter().rect_stroke(
                handle_rect,
                1.0,
                egui::Stroke::new(1.0, egui::Color32::WHITE),
                egui::epaint::StrokeKind::Outside,
            );
        }
    }

    /// Find which object is at the given position (relative to widget)
    fn find_object_at(&self, pos: egui::Pos2) -> Option<(ObjectId, egui::Rect)> {
        self.find_object_recursive(self.object, Point::default(), pos)
    }

    /// Find an object by its ID and return its rectangle
    fn find_object_by_id(&self, id: ObjectId, offset: Point<i16>) -> Option<(ObjectId, egui::Rect)> {
        self.find_object_by_id_recursive(self.object, offset, id)
    }

    fn find_object_by_id_recursive(
        &self,
        object: &Object,
        offset: Point<i16>,
        target_id: ObjectId,
    ) -> Option<(ObjectId, egui::Rect)> {
        // Check if this is the object we're looking for
        if object.id() == target_id {
            let (width, height) = self.pool.content_size(object);
            let rect = egui::Rect::from_min_size(
                egui::pos2(offset.x as f32, offset.y as f32),
                egui::vec2(width as f32, height as f32),
            );
            return Some((object.id(), rect));
        }

        // Check children
        match object {
            Object::DataMask(mask) => {
                for obj_ref in mask.object_refs.iter() {
                    if let Some(child) = self.pool.object_by_id(obj_ref.id) {
                        let child_offset = Point {
                            x: offset.x + obj_ref.offset.x,
                            y: offset.y + obj_ref.offset.y,
                        };
                        if let Some(result) = self.find_object_by_id_recursive(child, child_offset, target_id) {
                            return Some(result);
                        }
                    }
                }
            }
            Object::AlarmMask(mask) => {
                for obj_ref in mask.object_refs.iter() {
                    if let Some(child) = self.pool.object_by_id(obj_ref.id) {
                        let child_offset = Point {
                            x: offset.x + obj_ref.offset.x,
                            y: offset.y + obj_ref.offset.y,
                        };
                        if let Some(result) = self.find_object_by_id_recursive(child, child_offset, target_id) {
                            return Some(result);
                        }
                    }
                }
            }
            Object::Container(container) => {
                for obj_ref in container.object_refs.iter() {
                    if let Some(child) = self.pool.object_by_id(obj_ref.id) {
                        let child_offset = Point {
                            x: offset.x + obj_ref.offset.x,
                            y: offset.y + obj_ref.offset.y,
                        };
                        if let Some(result) = self.find_object_by_id_recursive(child, child_offset, target_id) {
                            return Some(result);
                        }
                    }
                }
            }
            _ => {}
        }

        None
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
