//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

use std::{
    cell::RefCell,
    collections::HashSet,
    sync::{Arc, Mutex},
    time::Duration,
};

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

pub(crate) fn preview_session(
    ui: &egui::Ui,
) -> Option<Arc<Mutex<crate::preview_session::PreviewSession>>> {
    ui.ctx()
        .data(|data| data.get_temp(egui::Id::new("vt_preview_session")))
}

/// Render from an isolated local playback session. The caller brackets each
/// displayed frame with `PreviewSession::begin_frame` and `finish_frame`.
/// No session lock is held while painting or recursively rendering children.
pub fn render_object_with_preview_session(
    ui: &mut egui::Ui,
    session: &Arc<Mutex<crate::preview_session::PreviewSession>>,
    object_id: ag_iso_stack::object_pool::ObjectId,
    position: Point<i16>,
    version: VtVersion,
    soft_key_size: (u16, u16),
) {
    let pool = match session.lock() {
        Ok(session) => session.pool().clone(),
        Err(_) => return,
    };
    let Some(object) = pool.object_by_id(object_id) else {
        return;
    };
    let key = egui::Id::new("vt_preview_session");
    let previous = ui.ctx().data_mut(|data| {
        let previous = data.get_temp::<Arc<Mutex<crate::preview_session::PreviewSession>>>(key);
        data.insert_temp(key, session.clone());
        previous
    });
    render_object_with_settings(ui, &pool, object, position, version, soft_key_size);
    ui.ctx().data_mut(|data| match previous {
        Some(previous) => data.insert_temp(key, previous),
        None => {
            data.remove::<Arc<Mutex<crate::preview_session::PreviewSession>>>(key);
        }
    });
}

/// Render a pool object using the requested target VT version and the legacy
/// 60 × 60 pixel Soft Key size. The settings apply to nested object rendering
/// and restore any enclosing render settings when this call returns.
pub fn render_object_with_version(
    ui: &mut egui::Ui,
    pool: &ObjectPool,
    object: &Object,
    position: Point<i16>,
    version: VtVersion,
) {
    render_object_with_settings(ui, pool, object, position, version, (60, 60));
}

/// Render an object with project VT and Soft Key settings. Soft Key dimensions
/// are in terminal pixels and bound auxiliary designator previews. Settings
/// are scoped to this call, inherited by nested rendering, then restored to
/// the previous values.
pub fn render_object_with_settings(
    ui: &mut egui::Ui,
    pool: &ObjectPool,
    object: &Object,
    position: Point<i16>,
    version: VtVersion,
    soft_key_size: (u16, u16),
) {
    // Offscreen Graphics Context draws can start during an interactive object
    // render. Each root needs its own ancestry so those snapshots do not look
    // like recursive references into the surrounding canvas.
    let _root =
        RenderRootGuard(RENDER_ANCESTRY.with(|active| std::mem::take(&mut *active.borrow_mut())));
    let previous = ui.ctx().data_mut(|data| {
        let previous = data.get_temp::<VtVersion>(render_version_id());
        data.insert_temp(render_version_id(), version);
        previous
    });
    let settings_key = egui::Id::new("vt_render_soft_key_size");
    let previous_settings = ui.ctx().data_mut(|data| {
        let previous = data.get_temp::<(u16, u16)>(settings_key);
        data.insert_temp(settings_key, soft_key_size);
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
    ui.ctx().data_mut(|data| match previous_settings {
        Some(previous) => data.insert_temp(settings_key, previous),
        None => {
            data.remove_temp::<(u16, u16)>(settings_key);
        }
    });
}

fn soft_key_size(ui: &egui::Ui) -> (u16, u16) {
    ui.ctx()
        .data(|data| data.get_temp::<(u16, u16)>(egui::Id::new("vt_render_soft_key_size")))
        .unwrap_or((60, 60))
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

struct RenderRootGuard(HashSet<u16>);

impl Drop for RenderRootGuard {
    fn drop(&mut self) {
        RENDER_ANCESTRY.with(|active| *active.borrow_mut() = std::mem::take(&mut self.0));
    }
}

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
        if preview_session(ui).is_some_and(|session| {
            session
                .lock()
                .is_ok_and(|session| session.is_hidden(self.id().value()))
        }) {
            return;
        }
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
            Object::AuxiliaryFunctionType1(o) => o.render(ui, pool, position),
            Object::AuxiliaryInputType1(o) => o.render(ui, pool, position),
            Object::ExternalObjectPointer(o) => o.render(ui, pool, position),
            Object::ScaledGraphic(o) => {
                crate::graphic_rendering::render_scaled(o, ui, pool, position)
            }
            Object::Animation(o) => o.render(ui, pool, position),
            Object::GraphicsContext(o) => o.render(ui, pool, position),
            Object::ExtendedInputAttributes(o) => (),
            Object::ColourMap(o) => (),
            Object::ObjectLabelReferenceList(o) => (),
            Object::ExternalObjectDefinition(o) => (),
            Object::ExternalReferenceName(o) => (),
            Object::ColourPalette(o) => (),
            Object::GraphicData(o) => (),
            Object::WorkingSetSpecialControls(o) => (),
            Object::WindowMask(_) | Object::KeyGroup(_) => {
                ui.painter().text(
                    ui.min_rect().min + egui::vec2(position.x as f32, position.y as f32),
                    egui::Align2::LEFT_TOP,
                    format!("Preview unavailable: {:?}", self.object_type()),
                    egui::FontId::proportional(12.0),
                    ui.visuals().warn_fg_color,
                );
            }
        }
        preview_activation(ui, self, position);
    }
}

fn preview_activation(ui: &mut egui::Ui, object: &Object, position: Point<i16>) {
    use ag_iso_stack::object_pool::object_attributes::Event;
    let Some(session) = preview_session(ui) else {
        return;
    };
    let size = match object {
        Object::Button(button) if !button.options.disabled => {
            egui::vec2(button.width as f32, button.height as f32)
        }
        Object::Key(_) => {
            let (w, h) = soft_key_size(ui);
            egui::vec2(w as f32, h as f32)
        }
        _ => return,
    };
    let rect = create_relative_rect(ui, position, size).intersect(ui.clip_rect());
    let id = ui.id().with((
        "preview_activation",
        object.id().value(),
        position.x,
        position.y,
    ));
    let response = ui.interact(rect, id, egui::Sense::click_and_drag());
    let pressed = response.is_pointer_button_down_on();
    let previous = ui.ctx().data_mut(|data| {
        let previous = data.get_temp::<bool>(id).unwrap_or(false);
        data.insert_temp(id, pressed);
        previous
    });
    let event = if pressed && !previous {
        Some(Event::OnKeyPress)
    } else if !pressed && previous {
        Some(Event::OnKeyRelease)
    } else {
        None
    };
    if let Some(event) = event {
        if let Ok(mut session) = session.lock() {
            let _ = session.trigger_key_event(object.id().value(), event == Event::OnKeyPress);
        }
        ui.ctx().request_repaint();
    }
}

pub(crate) trait Colorable {
    fn convert(&self) -> egui::Color32;
}

impl Colorable for Colour {
    fn convert(&self) -> egui::Color32 {
        egui::Color32::from_rgba_unmultiplied(self.r, self.g, self.b, self.a)
    }
}

// Helper function to lighten a color by a certain amount
fn lighten_color(color: egui::Color32, amount: f32) -> egui::Color32 {
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    let brighten = |value| (value as f32 + 255.0 * amount).min(255.0) as u8;
    egui::Color32::from_rgba_unmultiplied(brighten(r), brighten(g), brighten(b), a)
}

// Helper function to darken a color by a certain amount
fn darken_color(color: egui::Color32, amount: f32) -> egui::Color32 {
    let [r, g, b, a] = color.to_srgba_unmultiplied();
    let darken = |value| (value as f32 * (1.0 - amount)).max(0.0) as u8;
    egui::Color32::from_rgba_unmultiplied(darken(r), darken(g), darken(b), a)
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
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        render_designator(
            ui,
            pool,
            position,
            self.background_colour,
            &self.object_refs,
        );
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
        let face_inset = crate::button_geometry::face_inset(self, vt_version);
        let face_rect = rect.shrink(face_inset);

        let response = ui.interact(
            face_rect,
            ui.id().with(self.id.value()),
            egui::Sense::click(),
        );

        // Determine the current visual state
        // Priority: latched > pressed > hovered > normal
        let is_pressed_state = latched
            || (response.is_pointer_button_down_on() && !latchable)
            || preview_session(ui).is_some_and(|session| {
                session
                    .lock()
                    .is_ok_and(|session| session.button_is_pressed(self.id.value()))
            });
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

/// Working Set and auxiliary identifiers each occupy one Soft Key designator.
/// Their own origin also anchors pattern fills, independently of the mask.
fn render_designator(
    ui: &mut egui::Ui,
    pool: &ObjectPool,
    position: Point<i16>,
    background: u8,
    children: &Vec<ObjectRef>,
) {
    let (width, height) = soft_key_size(ui);
    let rect = create_relative_rect(ui, position, egui::vec2(width as f32, height as f32));
    ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
        ui.shrink_clip_rect(rect);
        ui.painter().rect_filled(
            rect,
            0.0,
            crate::effective_colour::colour(pool, background).convert(),
        );
        let key = egui::Id::new("vt_pattern_origin");
        let previous = ui.ctx().data_mut(|data| {
            let previous = data.get_temp::<egui::Pos2>(key);
            data.insert_temp(key, rect.min);
            previous
        });
        render_object_refs(ui, pool, children);
        ui.ctx().data_mut(|data| match previous {
            Some(origin) => data.insert_temp(key, origin),
            None => {
                data.remove_temp::<egui::Pos2>(key);
            }
        });
    });
}

impl RenderableObject for AuxiliaryFunctionType1 {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        render_designator(
            ui,
            pool,
            position,
            self.background_colour,
            &self.object_refs,
        );
    }
}

impl RenderableObject for AuxiliaryInputType1 {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        render_designator(
            ui,
            pool,
            position,
            self.background_colour,
            &self.object_refs,
        );
    }
}

impl RenderableObject for ExternalObjectPointer {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        // A standalone project has no validated external Working Set connection.
        // B.24 requires the local default in this case, including NULL = no drawing.
        if let Some(object) = pool.object_by_nullable_id(self.default_object_id) {
            object.render(ui, pool, position);
        }
    }
}

impl RenderableObject for Animation {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        let rect = create_relative_rect(
            ui,
            position,
            egui::vec2(self.width as f32, self.height as f32),
        );
        if !rect.intersect(ui.clip_rect()).is_positive() {
            return;
        }
        let child = if let Some(session) = preview_session(ui) {
            if self.enabled && self.refresh_interval != 0 {
                ui.ctx()
                    .request_repaint_after(Duration::from_millis(u64::from(self.refresh_interval)));
            }
            session
                .lock()
                .ok()
                .and_then(|mut session| session.animation_child(self.id.value()))
        } else {
            crate::preview_session::animation_static_child(self)
        };
        let Some(child) = child else {
            return;
        };
        if let Some(object) = pool.object_by_id(child.id) {
            ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
                ui.shrink_clip_rect(rect);
                object.render(ui, pool, child.offset);
            });
        }
    }
}

impl RenderableObject for GraphicsContext {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        use std::hash::{Hash, Hasher};
        let error = |ui: &egui::Ui, message: &str| {
            ui.painter().text(
                ui.max_rect().min + egui::vec2(position.x as f32, position.y as f32),
                egui::Align2::LEFT_TOP,
                message,
                egui::FontId::monospace(10.0),
                Color32::RED,
            );
        };
        let (canvas, limit) = if let Some(session) = preview_session(ui) {
            let Ok(mut session) = session.lock() else {
                return;
            };
            let limit = session.pixel_limit();
            let Some(canvas) = session.graphics_context(self.id.value()) else {
                return;
            };
            (canvas, limit)
        } else {
            let limit = crate::graphics_context_rendering::default_pixel_limit();
            match crate::graphics_context_rendering::GraphicsContextCanvas::try_from_object(
                self, limit,
            ) {
                Ok(canvas) => (canvas, limit),
                Err(message) => {
                    error(ui, &message);
                    return;
                }
            }
        };
        if canvas.viewport.2 == 0 || canvas.viewport.3 == 0 {
            return;
        }
        let rect = create_relative_rect(
            ui,
            position,
            egui::vec2(canvas.viewport.2 as f32, canvas.viewport.3 as f32),
        );
        if !rect.intersect(ui.clip_rect()).is_positive() {
            return;
        }
        let image = match canvas.try_viewport_image(pool, limit) {
            Ok(image) => image,
            Err(message) => {
                error(ui, &message);
                return;
            }
        };
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        image.size.hash(&mut hasher);
        image.pixels.hash(&mut hasher);
        let fingerprint = hasher.finish();
        let key = ui.id().with(("graphics_context_texture", self.id.value()));
        let cached = ui
            .ctx()
            .data(|data| data.get_temp::<(u64, egui::TextureHandle)>(key));
        let texture = if let Some((_, texture)) = cached.filter(|(hash, _)| *hash == fingerprint) {
            texture
        } else {
            let texture = ui.ctx().load_texture(
                format!("graphics_context_{}", self.id.value()),
                image,
                egui::TextureOptions::NEAREST,
            );
            ui.ctx()
                .data_mut(|data| data.insert_temp(key, (fingerprint, texture.clone())));
            texture
        };
        ui.painter()
            .with_clip_rect(rect.intersect(ui.clip_rect()))
            .image(
                texture.id(),
                rect,
                egui::Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
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
    // Tables B.18/B.23 test the effective raw value before offset/scaling.
    if options.display_zero_as_blank && raw == 0 {
        return String::new();
    }
    let value = (f64::from(raw) + f64::from(offset)) * f64::from(scale);
    if !value.is_finite() {
        return value.to_string();
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
pub(crate) fn paint_shape(
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
    // Mesh quads avoid antialiased seams between scanlines after fractional
    // canvas scaling.
    let mut fill_mesh = egui::Mesh::default();
    let mut border_mesh = egui::Mesh::default();
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
                            fill_mesh.add_colored_rect(
                                egui::Rect::from_min_max(
                                    egui::pos2(left, y as f32),
                                    egui::pos2(end, y as f32 + 1.0),
                                ),
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
                                fill_mesh.add_colored_rect(
                                    egui::Rect::from_min_size(
                                        egui::pos2(start as f32, y as f32),
                                        egui::vec2((x - start) as f32, 1.0),
                                    ),
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
                    border_mesh.add_colored_rect(
                        egui::Rect::from_min_max(
                            egui::pos2(left, y as f32),
                            egui::pos2(right, y as f32 + 1.0),
                        ),
                        line_colour,
                    );
                    run = Some((a, b));
                }
                None => run = Some((a, b)),
            }
        }
    }
    if !fill_mesh.is_empty() {
        painter.add(egui::Shape::mesh(fill_mesh));
    }
    if !border_mesh.is_empty() {
        painter.add(egui::Shape::mesh(border_mesh));
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

pub(crate) fn ellipse_path(rect: egui::Rect, kind: u8, start: u8, end: u8) -> Vec<egui::Pos2> {
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
            .map(|point| {
                // B.10.1 locates a point at the square brush's upper-left corner.
                // Preserve B.33 coordinates; the enclosing rectangle clips
                // any portion of the brush beyond its right/bottom edge.
                let width = f32::from(line.line_width);
                rect.min + egui::vec2(point.x as f32 + width * 0.5, point.y as f32 + width * 0.5)
            })
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
        crate::numeric_rendering::render_meter(self, ui, pool, position, render_version(ui));
    }
}

impl RenderableObject for OutputLinearBarGraph {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        crate::numeric_rendering::render_linear_bar_graph(
            self,
            ui,
            pool,
            position,
            render_version(ui),
        );
    }
}

impl RenderableObject for OutputArchedBarGraph {
    fn render(&self, ui: &mut egui::Ui, pool: &ObjectPool, position: Point<i16>) {
        crate::numeric_rendering::render_arched_bar_graph(
            self,
            ui,
            pool,
            position,
            render_version(ui),
        );
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
        let phase = ui.input(|input| input.time).rem_euclid(1.0);
        if self.options.flashing {
            ui.ctx()
                .request_repaint_after(Duration::from_secs_f64(if phase < 0.5 {
                    0.5 - phase
                } else {
                    1.0 - phase
                }));
            if phase >= 0.5 {
                return;
            }
        }

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
        let (width, height) = soft_key_size(ui);
        let rect = create_relative_rect(ui, position, egui::vec2(width as f32, height as f32));

        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(rect));
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
        let (width, height) = soft_key_size(ui);
        let rect = create_relative_rect(ui, position, egui::vec2(width as f32, height as f32));

        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(rect));
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
        let (width, height) = soft_key_size(ui);
        let rect = create_relative_rect(ui, position, egui::vec2(width as f32, height as f32));

        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(rect));
            if self.pointer_type != 2 && self.auxiliary_object_id.0.is_none() {
                return;
            }
            // Determine what to display based on pointer_type
            match self.pointer_type {
                0 => {
                    // Pointer type 0: Points to Auxiliary Object referenced by auxiliary_object_id
                    // Display the auxiliary object designator
                    if let Some(aux_obj_id) = self.auxiliary_object_id.0 {
                        match pool.object_by_id(aux_obj_id) {
                            Some(aux @ Object::AuxiliaryFunctionType2(_))
                            | Some(aux @ Object::AuxiliaryInputType2(_)) => {
                                aux.render(ui, pool, Point { x: 0, y: 0 });
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
    use ag_iso_stack::object_pool::{NullableObjectId, ObjectType};

    #[test]
    fn numeric_zero_blank_uses_raw_local_or_variable_value_before_scaling() {
        use ag_iso_stack::object_pool::{object_attributes::NumberOptions, ObjectId};
        let options = NumberOptions {
            transparent: false,
            display_leading_zeros: false,
            display_zero_as_blank: true,
            truncate: false,
        };
        for format in [FormatType::Decimal, FormatType::Exponential] {
            assert_eq!(format_number(0, 5, 2.0, 0, format, options), "");
            assert!(!format_number(1, -1, 1.0, 0, format, options).is_empty());
            assert!(!format_number(1, 0, 0.0, 0, format, options).is_empty());
        }
        let mut pool = ObjectPool::new();
        let mut font = crate::default_object(ObjectType::FontAttributes);
        *font.mut_id() = ObjectId::new(2).unwrap();
        pool.add(font);
        pool.add(Object::NumberVariable(NumberVariable {
            id: ObjectId::new(3).unwrap(),
            value: 0,
        }));
        for kind in [ObjectType::InputNumber, ObjectType::OutputNumber] {
            let mut object = crate::default_object(kind);
            match &mut object {
                Object::InputNumber(o) => {
                    o.width = 80;
                    o.height = 30;
                    o.options = options;
                    o.font_attributes = NullableObjectId::new(2);
                    o.variable_reference = NullableObjectId::new(3);
                    o.value = 1;
                    o.offset = -1;
                    o.scale = 1.0;
                }
                Object::OutputNumber(o) => {
                    o.width = 80;
                    o.height = 30;
                    o.options = options;
                    o.font_attributes = NullableObjectId::new(2);
                    o.variable_reference = NullableObjectId::new(3);
                    o.value = 1;
                    o.offset = -1;
                    o.scale = 1.0;
                }
                _ => unreachable!(),
            }
            for raw in [0, 1] {
                if let Some(Object::NumberVariable(variable)) =
                    pool.object_mut_by_id(ObjectId::new(3).unwrap())
                {
                    variable.value = raw;
                }
                let text: String = frame(&pool, Some(&object), 0.0)
                    .shapes
                    .into_iter()
                    .filter_map(|shape| match shape.shape {
                        egui::Shape::Text(text) => Some(text.galley.text().to_owned()),
                        _ => None,
                    })
                    .collect();
                assert_eq!(text, if raw == 0 { "" } else { "0" }, "{kind:?}");
            }
        }
    }

    fn frame(pool: &ObjectPool, object: Option<&Object>, time: f64) -> egui::FullOutput {
        frame_at(pool, object, time, Point::default())
    }

    fn frame_at(
        pool: &ObjectPool,
        object: Option<&Object>,
        time: f64,
        position: Point<i16>,
    ) -> egui::FullOutput {
        let ctx = egui::Context::default();
        ctx.run(
            egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(100.0, 100.0),
                )),
                time: Some(time),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    if let Some(object) = object {
                        render_object_with_settings(
                            ui,
                            pool,
                            object,
                            position,
                            VtVersion::Version6,
                            (20, 15),
                        );
                    }
                });
            },
        )
    }

    #[test]
    fn working_set_designator_has_its_own_origin_bounds_and_palette_alpha() {
        let mut working_set = crate::default_object(ObjectType::WorkingSet);
        if let Object::WorkingSet(ws) = &mut working_set {
            ws.background_colour = 12;
        }
        let pool = ObjectPool::default();
        let origin = frame_at(&pool, Some(&working_set), 0.0, Point { x: 9, y: 11 });
        let rectangles: Vec<_> = origin
            .shapes
            .iter()
            .filter_map(|shape| {
                if let egui::Shape::Rect(rectangle) = &shape.shape {
                    (rectangle.fill == Color32::RED).then_some((rectangle.rect, shape.clip_rect))
                } else {
                    None
                }
            })
            .collect();
        assert_eq!(rectangles.len(), 1);
        assert_eq!(rectangles[0].0.min, egui::pos2(17.0, 19.0));
        assert_eq!(rectangles[0].0.size(), egui::vec2(20.0, 15.0));
        assert_eq!(rectangles[0].1, rectangles[0].0);

        let mut colour = Colour::BLACK;
        colour.a = 128;
        assert_eq!(
            colour.convert(),
            Color32::from_rgba_unmultiplied(0, 0, 0, 128)
        );
    }

    #[test]
    fn external_pointer_draws_local_default_and_null_draws_nothing() {
        let mut working_set = crate::default_object(ObjectType::WorkingSet);
        *working_set.mut_id() = ag_iso_stack::object_pool::ObjectId::new(2).unwrap();
        let mut pointer = crate::default_object(ObjectType::ExternalObjectPointer);
        *pointer.mut_id() = ag_iso_stack::object_pool::ObjectId::new(1).unwrap();
        if let Object::ExternalObjectPointer(pointer) = &mut pointer {
            pointer.default_object_id = NullableObjectId::new(2);
        }
        let mut pool = ObjectPool::default();
        pool.add(working_set.clone());
        let rectangles = |out: egui::FullOutput| {
            out.shapes
                .into_iter()
                .filter_map(|s| {
                    if let egui::Shape::Rect(r) = s.shape {
                        Some((s.clip_rect, r.rect, r.fill))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(
            rectangles(frame(&pool, Some(&working_set), 0.0)),
            rectangles(frame(&pool, Some(&pointer), 0.0))
        );
        if let Object::ExternalObjectPointer(pointer) = &mut pointer {
            pointer.default_object_id = NullableObjectId::NULL;
        }
        assert_eq!(
            rectangles(frame(&pool, None, 0.0)),
            rectangles(frame(&pool, Some(&pointer), 0.0))
        );
    }

    #[test]
    fn polygon_boundary_keeps_the_complete_square_brush() {
        let mut polygon = crate::default_object(ObjectType::OutputPolygon);
        if let Object::OutputPolygon(polygon) = &mut polygon {
            polygon.width = 10;
            polygon.height = 10;
            polygon.line_attributes = NullableObjectId::new(2);
            polygon.fill_attributes = NullableObjectId::NULL;
            polygon.points = vec![
                Point { x: 0, y: 0 },
                Point { x: 8, y: 0 },
                Point { x: 8, y: 8 },
                Point { x: 0, y: 8 },
            ];
        }
        let mut line = crate::default_object(ObjectType::LineAttributes);
        *line.mut_id() = ag_iso_stack::object_pool::ObjectId::new(2).unwrap();
        if let Object::LineAttributes(line) = &mut line {
            line.line_width = 2;
            line.line_art = 65535;
            line.line_colour = 12;
        }
        let mut pool = ObjectPool::default();
        pool.add(line);
        let rendered = frame(&pool, Some(&polygon), 0.0);
        let red_meshes: Vec<_> = rendered
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Mesh(mesh)
                    if mesh
                        .vertices
                        .iter()
                        .any(|vertex| vertex.color == Color32::RED) =>
                {
                    Some(mesh)
                }
                _ => None,
            })
            .collect();
        assert_eq!(red_meshes.len(), 1);
        let red_positions: Vec<_> = red_meshes[0]
            .vertices
            .iter()
            .filter(|vertex| vertex.color == Color32::RED)
            .map(|vertex| vertex.pos)
            .collect();
        let mut bounds = egui::Rect::NOTHING;
        for point in red_positions {
            bounds.extend_with(point);
        }
        // The path endpoints are inset by half the 2px brush, whose complete
        // footprint restores the polygon's full 10px enclosing extent.
        assert_eq!(bounds.size(), egui::vec2(10.0, 10.0));
    }

    #[test]
    fn polygon_fill_mesh_has_no_scanline_gaps_at_fractional_zoom() {
        let mut line_object = crate::default_object(ObjectType::LineAttributes);
        *line_object.mut_id() = ag_iso_stack::object_pool::ObjectId::new(2).unwrap();
        if let Object::LineAttributes(line) = &mut line_object {
            line.line_width = 0;
        }
        let mut fill_object = crate::default_object(ObjectType::FillAttributes);
        *fill_object.mut_id() = ag_iso_stack::object_pool::ObjectId::new(3).unwrap();
        if let Object::FillAttributes(fill) = &mut fill_object {
            fill.fill_type = 2;
            fill.fill_colour = 10;
        }
        let mut pool = ObjectPool::default();
        pool.add(line_object);
        pool.add(fill_object.clone());
        let Object::FillAttributes(fill) = &fill_object else {
            unreachable!()
        };
        let polygon_points = [
            egui::pos2(10.0, 10.0),
            egui::pos2(50.0, 10.0),
            egui::pos2(50.0, 50.0),
            egui::pos2(10.0, 50.0),
            egui::pos2(10.0, 10.0),
        ];
        let line = match pool.object_by_id(ag_iso_stack::object_pool::ObjectId::new(2).unwrap()) {
            Some(Object::LineAttributes(line)) => line,
            _ => unreachable!(),
        };
        let expected = crate::effective_colour::colour(&pool, 10).convert();

        for zoom in [0.5, 0.9, 0.95, 1.0, 1.25] {
            let offset = egui::vec2(0.37, 0.61);
            let transform = egui::emath::TSTransform::new(offset, zoom);
            let image = crate::graphics_context_raster::rasterize_paint(80, 80, 6400, |ui| {
                ui.with_visual_transform(transform, |ui| {
                    paint_shape(
                        ui,
                        &pool,
                        egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(80.0, 80.0)),
                        &polygon_points,
                        line,
                        Some(&fill_object),
                        0,
                    );
                });
            })
            .unwrap();
            // Stay well inside the outline and sample every rendered row;
            // independently antialiased scanline rectangles used to leave
            // background pixels here for fractional zooms.
            let top = (transform * egui::pos2(16.0, 16.0)).y.ceil() as usize;
            let bottom = (transform * egui::pos2(44.0, 44.0)).y.floor() as usize;
            let left = (transform * egui::pos2(16.0, 16.0)).x.ceil() as usize;
            let right = (transform * egui::pos2(44.0, 44.0)).x.floor() as usize;
            for y in top..=bottom {
                for x in left..=right {
                    assert_eq!(
                        image[(x, y)],
                        expected,
                        "gap at zoom={zoom}, pixel=({x},{y})"
                    );
                }
            }
        }
    }

    #[test]
    fn picture_flashing_and_auxiliary_designators_respect_render_settings() {
        let mut picture = crate::default_object(ObjectType::PictureGraphic);
        if let Object::PictureGraphic(picture) = &mut picture {
            picture.id = ag_iso_stack::object_pool::ObjectId::new(1).unwrap();
            picture.width = 1;
            picture.actual_width = 1;
            picture.actual_height = 1;
            picture.data = vec![0];
            picture.options.flashing = true;
        }
        let mut pool = ObjectPool::default();
        pool.add(picture.clone());
        let visible = frame(&pool, Some(&picture), 0.25);
        let hidden = frame(&pool, Some(&picture), 0.75);
        assert!(visible.shapes.len() > hidden.shapes.len());

        let mut auxiliary = crate::default_object(ObjectType::AuxiliaryFunctionType2);
        if let Object::AuxiliaryFunctionType2(auxiliary) = &mut auxiliary {
            auxiliary.id = ag_iso_stack::object_pool::ObjectId::new(2).unwrap();
            auxiliary.object_refs = vec![ObjectRef {
                id: ag_iso_stack::object_pool::ObjectId::new(4).unwrap(),
                offset: Point { x: 0, y: 0 },
            }];
        }
        let mut button = crate::default_object(ObjectType::Button);
        if let Object::Button(button) = &mut button {
            button.id = ag_iso_stack::object_pool::ObjectId::new(4).unwrap();
            button.width = 50;
            button.height = 40;
        }
        let mut direct_pool = ObjectPool::default();
        direct_pool.add(button.clone());
        let direct = frame(&direct_pool, Some(&auxiliary), 0.25);
        let mut pointer = crate::default_object(ObjectType::AuxiliaryControlDesignatorType2);
        if let Object::AuxiliaryControlDesignatorType2(pointer) = &mut pointer {
            pointer.id = ag_iso_stack::object_pool::ObjectId::new(3).unwrap();
            pointer.auxiliary_object_id = NullableObjectId::new(2);
        }
        let mut aux_pool = ObjectPool::default();
        aux_pool.add(button);
        aux_pool.add(auxiliary.clone());
        aux_pool.add(pointer.clone());
        let referenced = frame(&aux_pool, Some(&pointer), 0.25);
        let rect_shapes = |output: &egui::FullOutput| {
            output
                .shapes
                .iter()
                .filter_map(|shape| match &shape.shape {
                    egui::Shape::Rect(rect) => Some((shape.clip_rect, rect.rect, rect.fill)),
                    _ => None,
                })
                .collect::<Vec<_>>()
        };
        assert_eq!(rect_shapes(&direct), rect_shapes(&referenced));
        assert!(referenced.shapes.iter().any(|shape| match &shape.shape {
            egui::Shape::Rect(rect) => {
                rect.rect.size() == egui::vec2(50.0, 40.0)
                    && shape.clip_rect.size() == egui::vec2(20.0, 15.0)
            }
            _ => false,
        }));

        let empty = frame(&aux_pool, None, 0.25);
        for pointer_type in [0, 1, 3] {
            if let Object::AuxiliaryControlDesignatorType2(pointer) = &mut pointer {
                pointer.pointer_type = pointer_type;
                pointer.auxiliary_object_id = NullableObjectId::NULL;
            }
            let null_pointer = frame(&aux_pool, Some(&pointer), 0.25);
            assert_eq!(null_pointer.shapes.len(), empty.shapes.len());
        }
    }
}
