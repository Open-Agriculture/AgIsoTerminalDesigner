//! Transient selection shared by shape property controls and canvas handles.
use crate::EditorProject;
use ag_iso_stack::object_pool::object_attributes::Point;
use egui::{Context, Id};

#[derive(Clone, Default)]
struct Selection {
    editing: bool,
    point: Option<usize>,
}

fn key(project: &EditorProject, object_id: u16) -> Id {
    Id::new(("shape_point_editing", project.ui_session_id(), object_id))
}

pub fn editing(ctx: &Context, project: &EditorProject, object_id: u16) -> bool {
    ctx.data(|data| data.get_temp::<Selection>(key(project, object_id)))
        .is_some_and(|state| state.editing)
}

pub fn selected_point(ctx: &Context, project: &EditorProject, object_id: u16) -> Option<usize> {
    ctx.data(|data| data.get_temp::<Selection>(key(project, object_id)))
        .and_then(|state| state.point)
}

pub fn select_point(ctx: &Context, project: &EditorProject, object_id: u16, index: usize) {
    ctx.data_mut(|data| {
        let key = key(project, object_id);
        let mut state = data.get_temp::<Selection>(key).unwrap_or_default();
        state.point = Some(index);
        data.insert_temp(key, state);
    });
}

pub fn stop_editing(ctx: &Context, project: &EditorProject, object_id: u16) {
    set_editing(ctx, project, object_id, false);
}

pub fn set_editing(ctx: &Context, project: &EditorProject, object_id: u16, editing: bool) {
    ctx.data_mut(|data| {
        let key = key(project, object_id);
        let mut state = data.get_temp::<Selection>(key).unwrap_or_default();
        state.editing = editing;
        data.insert_temp(key, state);
    });
}

/// Coordinate for an insertion slot, including the closing edge of a closed polygon.
pub(crate) fn insertion_point(
    points: &[Point<u16>],
    slot: usize,
    open: bool,
    width: u16,
    height: u16,
) -> Point<u16> {
    let count = points.len();
    if count == 0 {
        return Point { x: 0, y: 0 };
    }
    if count > 1 && (!open || slot > 0 && slot < count) {
        let a = &points[(slot + count - 1) % count];
        let b = &points[slot % count];
        return Point {
            x: ((u32::from(a.x) + u32::from(b.x)) / 2) as u16,
            y: ((u32::from(a.y) + u32::from(b.y)) / 2) as u16,
        };
    }
    let point = &points[slot.saturating_sub(1).min(count - 1)];
    fn offset(value: u16, extent: u16) -> u16 {
        let max = extent.saturating_sub(1);
        let value = value.min(max);
        if max - value >= 8 {
            value + 8
        } else if value >= 8 {
            value - 8
        } else if max - value >= value {
            max
        } else {
            0
        }
    }
    Point {
        x: offset(point.x, width),
        y: offset(point.y, height),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn insertion_uses_edges_and_open_end_offsets_without_overflow() {
        let points = [Point { x: 2, y: 4 }, Point { x: 14, y: 18 }];
        assert_eq!(
            insertion_point(&points, 1, true, 30, 30),
            Point { x: 8, y: 11 }
        );
        assert_eq!(
            insertion_point(&points, 2, false, 30, 30),
            Point { x: 8, y: 11 }
        );
        assert_eq!(
            insertion_point(&points, 0, true, 30, 30),
            Point { x: 10, y: 12 }
        );
        assert_eq!(
            insertion_point(&points, 2, true, 20, 20),
            Point { x: 6, y: 10 }
        );
        assert_eq!(insertion_point(&[], 0, true, 0, 0), Point { x: 0, y: 0 });
        assert_eq!(
            insertion_point(&points[..1], 1, false, 1, 1),
            Point { x: 0, y: 0 }
        );
        let large = [Point {
            x: u16::MAX,
            y: u16::MAX,
        }; 2];
        assert_eq!(insertion_point(&large, 1, false, 0, 0), large[0]);
    }
}
