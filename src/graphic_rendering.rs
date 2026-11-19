//! ISO 11783-6 B.28 Scaled Graphic presentation.
use std::collections::HashSet;

use ag_iso_stack::object_pool::{
    object::{Object, ScaledGraphic},
    object_attributes::Point,
    ObjectPool,
};
use egui::{Color32, Rect, Vec2};

/// Follow the B.76 pointer chain without allowing malformed drafts to recurse.
fn graphic<'a>(pool: &'a ObjectPool, object: &ScaledGraphic) -> Option<&'a Object> {
    let mut id = object.value.0?;
    let mut seen = HashSet::new();
    while seen.insert(id) {
        match pool.object_by_id(id)? {
            value @ (Object::PictureGraphic(_) | Object::GraphicData(_)) => return Some(value),
            Object::ObjectPointer(pointer) => id = pointer.value.0?,
            _ => return None,
        }
    }
    None
}

fn image_rect(field: Rect, source: Vec2, scale_type: u8) -> Option<Rect> {
    if !field.is_positive() || source.x <= 0.0 || source.y <= 0.0 {
        return None;
    }
    let size = match scale_type & 7 {
        0 => source,
        1 => source * (field.width() / source.x),
        2 => source * (field.height() / source.y),
        3 => field.size(),
        4 => source * (field.width() / source.x).min(field.height() / source.y),
        _ => return None,
    };
    let justify = |code, extra| match code {
        0 => Some(0.0),
        1 => Some(extra * 0.5),
        2 => Some(extra),
        _ => None,
    };
    let offset = egui::vec2(
        justify((scale_type >> 3) & 3, field.width() - size.x)?,
        justify((scale_type >> 5) & 3, field.height() - size.y)?,
    );
    Some(Rect::from_min_size(field.min + offset, size))
}

pub(crate) fn render_scaled(
    object: &ScaledGraphic,
    ui: &mut egui::Ui,
    pool: &ObjectPool,
    position: Point<i16>,
) {
    let field = Rect::from_min_size(
        ui.max_rect().min + egui::vec2(position.x as f32, position.y as f32),
        egui::vec2(object.width as f32, object.height as f32),
    );
    if !field.intersect(ui.clip_rect()).is_positive() {
        return;
    }
    if object.options.flashing {
        let phase = ui.input(|i| i.time).rem_euclid(1.0);
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_secs_f64(if phase < 0.5 {
                0.5 - phase
            } else {
                1.0 - phase
            }));
        if phase >= 0.5 {
            return;
        }
    }
    let Some(source) = graphic(pool, object) else {
        return;
    };
    let texture = match source {
        Object::PictureGraphic(picture) => {
            crate::graphics::preview::picture_texture(ui.ctx(), picture, pool)
        }
        Object::GraphicData(data) => crate::graphics::preview::graphic_data_texture(ui.ctx(), data),
        _ => None,
    };
    let Some(texture) = texture else {
        return;
    };
    let Some(rect) = image_rect(field, texture.size_vec2(), object.scale_type) else {
        return;
    };
    ui.painter()
        .with_clip_rect(field.intersect(ui.clip_rect()))
        .image(
            texture.id(),
            rect,
            Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
            Color32::WHITE,
        );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scaling_and_justification_use_raw_dimensions_and_clip_separately() {
        let field = Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(100.0, 60.0));
        let raw = egui::vec2(40.0, 20.0);
        let sizes = [
            raw,
            egui::vec2(100.0, 50.0),
            egui::vec2(120.0, 60.0),
            field.size(),
            egui::vec2(100.0, 50.0),
        ];
        for (mode, expected) in sizes.into_iter().enumerate() {
            let rect = image_rect(field, raw, mode as u8 | (2 << 3) | (2 << 5)).unwrap();
            assert_eq!(rect.size(), expected);
            assert_eq!(rect.max, field.max);
        }
        assert!(image_rect(field, raw, 5).is_none());
    }
}
