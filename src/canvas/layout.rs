//! Coordinate calculations, layout commands, snapping, and canvas overlays.
use super::{
    edits::{position, CanvasEdit, PlacementTarget},
    geometry::{Scene, SceneItem},
};
use crate::EditorProject;
use ag_iso_stack::object_pool::ObjectId;
use egui::{Color32, Modifiers, Pos2, Rect, Stroke, Vec2};

pub(super) fn resolves_to(
    pool: &ag_iso_stack::object_pool::ObjectPool,
    id: u16,
    target: u16,
) -> bool {
    let mut id = id;
    let mut seen = std::collections::HashSet::new();
    while seen.insert(id) {
        if id == target {
            return true;
        }
        match ObjectId::new(id).ok().and_then(|id| pool.object_by_id(id)) {
            Some(ag_iso_stack::object_pool::object::Object::ObjectPointer(pointer)) => {
                if let Some(next) = pointer.value.0 {
                    id = next.value()
                } else {
                    return false;
                }
            }
            _ => return false,
        }
    }
    false
}

pub(super) fn dimensions(project: &EditorProject, item: &SceneItem) -> (Option<u16>, Option<u16>) {
    if !matches!(item.target, Some(PlacementTarget::Primitive { .. })) {
        return (None, None);
    }
    let Some(object) = ObjectId::new(item.object_id)
        .ok()
        .and_then(|id| project.get_pool().object_by_id(id))
    else {
        return (None, None);
    };
    let get = |name| {
        crate::object_properties::get_object_property(object, name)
            .ok()
            .and_then(|v| v.as_u64())
            .and_then(|v| u16::try_from(v).ok())
    };
    (
        get(crate::object_properties::PropertyKey::Width),
        get(crate::object_properties::PropertyKey::Height),
    )
}

pub(super) fn bounds(items: &[SceneItem]) -> Option<Rect> {
    items.iter().map(|i| i.rect).reduce(|a, b| a.union(b))
}

pub(super) fn root_items(items: Vec<SceneItem>) -> Vec<SceneItem> {
    // Moving a parent already carries its children; deduplicate aliased targets too.
    let mut out: Vec<SceneItem> = vec![];
    for item in &items {
        if items
            .iter()
            .any(|p| p.path.len() < item.path.len() && item.path.starts_with(&p.path))
        {
            continue;
        }
        if item.target.is_some() && !out.iter().any(|p| p.target == item.target) {
            out.push(item.clone());
        }
    }
    out
}

pub(super) fn same_parent(items: &[SceneItem]) -> bool {
    let Some(first) = items.first() else {
        return false;
    };
    let parent = &first.path[..first.path.len().saturating_sub(1)];
    items
        .iter()
        .all(|i| &i.path[..i.path.len().saturating_sub(1)] == parent)
}

pub(super) fn constrain(delta: Vec2, shift: bool) -> Vec2 {
    if !shift {
        delta
    } else if delta.x.abs() >= delta.y.abs() {
        egui::vec2(delta.x, 0.0)
    } else {
        egui::vec2(0.0, delta.y)
    }
}
pub(super) fn move_edits(
    project: &EditorProject,
    items: &[SceneItem],
    delta: Vec2,
    duplicate: bool,
) -> Vec<CanvasEdit> {
    let mut delta = delta.round();
    // Clamp a group delta, rather than individual objects, to preserve spacing.
    for item in items {
        if let Some((x, y)) = item.target.as_ref().and_then(|t| position(project, t)) {
            delta.x = delta
                .x
                .clamp(i16::MIN as f32 - x as f32, i16::MAX as f32 - x as f32);
            delta.y = delta
                .y
                .clamp(i16::MIN as f32 - y as f32, i16::MAX as f32 - y as f32);
        }
    }
    if !duplicate && delta == Vec2::ZERO {
        return vec![];
    }
    if duplicate {
        // A duplicate creates a new placement, so it must fit fully even when
        // the source placement already has legacy overflow.
        let mut min_x = i16::MIN as f32;
        let mut max_x = i16::MAX as f32;
        let mut min_y = i16::MIN as f32;
        let mut max_y = i16::MAX as f32;
        for item in items {
            if item
                .target
                .as_ref()
                .and_then(|t| position(project, t))
                .is_none()
            {
                continue;
            }
            min_x = min_x.max(item.parent_rect.left() - item.rect.left());
            max_x = max_x.min(item.parent_rect.right() - item.rect.right());
            min_y = min_y.max(item.parent_rect.top() - item.rect.top());
            max_y = max_y.min(item.parent_rect.bottom() - item.rect.bottom());
        }
        if min_x > max_x || min_y > max_y {
            return vec![];
        }
        delta.x = delta.x.clamp(min_x.ceil(), max_x.floor());
        delta.y = delta.y.clamp(min_y.ceil(), max_y.floor());
    } else {
        // First clamp against each visible placement's parent. A shared parent
        // can occur in other masks or inherited viewports, so the shared
        // validator below still checks the resulting group delta.
        let mut min_x = i16::MIN as f32;
        let mut max_x = i16::MAX as f32;
        let mut min_y = i16::MIN as f32;
        let mut max_y = i16::MAX as f32;
        for item in items {
            if item
                .target
                .as_ref()
                .and_then(|t| position(project, t))
                .is_none()
            {
                continue;
            }
            let parent = crate::layout_containment::Bounds {
                x: item.parent_rect.left().round() as i32,
                y: item.parent_rect.top().round() as i32,
                width: item.parent_rect.width().round() as i32,
                height: item.parent_rect.height().round() as i32,
            };
            let origin = (
                item.rect.left().round() as i32,
                item.rect.top().round() as i32,
            );
            let size = (
                item.rect.width().round() as i32,
                item.rect.height().round() as i32,
            );
            let range = crate::layout_containment::allowed_origin_range(parent, size, origin, true);
            min_x = min_x.max((range.min_x - origin.0) as f32);
            max_x = max_x.min((range.max_x - origin.0) as f32);
            min_y = min_y.max((range.min_y - origin.1) as f32);
            max_y = max_y.min((range.max_y - origin.1) as f32);
        }
        if min_x > max_x || min_y > max_y {
            return vec![];
        }
        delta.x = delta.x.clamp(min_x.ceil(), max_x.floor());
        delta.y = delta.y.clamp(min_y.ceil(), max_y.floor());
    }
    let make_edits = |delta: Vec2| {
        items
            .iter()
            .filter_map(|i| {
                let target = i.target.clone()?;
                let (x, y) = position(project, &target)?;
                let x = (x as f32 + delta.x) as i16;
                let y = (y as f32 + delta.y) as i16;
                Some(if duplicate {
                    CanvasEdit::Duplicate { target, x, y }
                } else {
                    CanvasEdit::Move { target, x, y }
                })
            })
            .collect::<Vec<_>>()
    };
    let proposed = make_edits(delta);
    if valid_canvas_edits(project, &proposed) {
        return proposed;
    }

    // Movement constraints are independent on each axis. Clamp both axes
    // separately so reaching one parent edge does not stop movement toward
    // another edge sooner than necessary.
    let clamp_axis = |requested: f32, horizontal: bool| {
        if requested == 0.0 {
            return 0.0;
        }
        let direction = requested.signum();
        let mut low = 0i32;
        let mut high = requested.abs() as i32;
        while low < high {
            let distance = low + (high - low + 1) / 2;
            let axis_delta = if horizontal {
                Vec2::new(direction * distance as f32, 0.0)
            } else {
                Vec2::new(0.0, direction * distance as f32)
            };
            if valid_canvas_edits(project, &make_edits(axis_delta)) {
                low = distance;
            } else {
                high = distance - 1;
            }
        }
        direction * low as f32
    };
    let clamped = Vec2::new(clamp_axis(delta.x, true), clamp_axis(delta.y, false)).round();
    let clamped_edits = make_edits(clamped);
    if valid_canvas_edits(project, &clamped_edits) {
        return clamped_edits;
    }

    // Keep a conservative fallback for geometry whose containment changes
    // across nested occurrences in a way that couples both axes.
    let mut low = 0.0;
    let mut high = 1.0;
    let mut best = make_edits(Vec2::ZERO);
    for _ in 0..16 {
        let fraction = (low + high) * 0.5;
        let candidate = make_edits(delta * fraction);
        if valid_canvas_edits(project, &candidate) {
            low = fraction;
            best = candidate;
        } else {
            high = fraction;
        }
    }
    if valid_canvas_edits(project, &best) {
        best
    } else {
        vec![]
    }
}

fn valid_canvas_edits(project: &EditorProject, edits: &[CanvasEdit]) -> bool {
    let mut preview = project.canvas_preview_copy();
    super::edits::apply(&mut preview, "Canvas containment preview", edits).is_ok()
}

pub(super) fn align_edits(
    project: &EditorProject,
    items: &[SceneItem],
    action: usize,
    parent: bool,
) -> Vec<CanvasEdit> {
    let Some(group) = bounds(items) else {
        return vec![];
    };
    if !same_parent(items) {
        return vec![];
    }
    let reference = if parent || items.len() == 1 {
        items[0].parent_rect
    } else {
        group
    };
    if action >= 6 {
        let horizontal = action == 6;
        let mut sorted = items.to_vec();
        sorted.sort_by(|a, b| {
            if horizontal {
                a.rect.left().total_cmp(&b.rect.left())
            } else {
                a.rect.top().total_cmp(&b.rect.top())
            }
        });
        let total: f32 = sorted
            .iter()
            .map(|i| {
                if horizontal {
                    i.rect.width()
                } else {
                    i.rect.height()
                }
            })
            .sum();
        let span = if horizontal {
            group.width()
        } else {
            group.height()
        };
        let gap = (span - total) / (items.len() - 1) as f32;
        let mut cursor = if horizontal {
            group.left()
        } else {
            group.top()
        };
        let mut out = vec![];
        for item in sorted {
            let old = if horizontal {
                item.rect.left()
            } else {
                item.rect.top()
            };
            out.extend(move_edits(
                project,
                &[item.clone()],
                if horizontal {
                    egui::vec2(cursor - old, 0.0)
                } else {
                    egui::vec2(0.0, cursor - old)
                },
                false,
            ));
            cursor += if horizontal {
                item.rect.width()
            } else {
                item.rect.height()
            };
            cursor += gap;
        }
        return out;
    }
    let delta = |r: Rect| match action {
        0 => egui::vec2(reference.left() - r.left(), 0.0),
        1 => egui::vec2(reference.center().x - r.center().x, 0.0),
        2 => egui::vec2(reference.right() - r.right(), 0.0),
        3 => egui::vec2(0.0, reference.top() - r.top()),
        4 => egui::vec2(0.0, reference.center().y - r.center().y),
        _ => egui::vec2(0.0, reference.bottom() - r.bottom()),
    };
    if parent {
        move_edits(project, items, delta(group), false)
    } else {
        items
            .iter()
            .flat_map(|i| move_edits(project, &[i.clone()], delta(i.rect), false))
            .collect()
    }
}

pub(super) fn handles(dim: (Option<u16>, Option<u16>)) -> Vec<Vec2> {
    let (w, h) = dim;
    let mut out = vec![];
    for x in [-1.0, 0.0, 1.0] {
        for y in [-1.0, 0.0, 1.0] {
            if (x != 0.0 || y != 0.0) && (x == 0.0 || w.is_some()) && (y == 0.0 || h.is_some()) {
                out.push(egui::vec2(x, y));
            }
        }
    }
    out
}
pub(super) fn handle_pos(rect: Rect, h: Vec2) -> Pos2 {
    rect.center() + rect.size() * h * 0.5
}
pub(super) fn resize_handle(
    item: &SceneItem,
    p: Pos2,
    origin: Pos2,
    zoom: f32,
    dim: (Option<u16>, Option<u16>),
) -> Option<Vec2> {
    let rect = Rect::from_min_max(
        origin + item.rect.min.to_vec2() * zoom,
        origin + item.rect.max.to_vec2() * zoom,
    );
    handles(dim)
        .into_iter()
        .find(|h| handle_pos(rect, *h).distance(p) <= 6.0)
}
pub(super) fn handle_cursor(h: Vec2) -> egui::CursorIcon {
    if h.x == 0.0 {
        egui::CursorIcon::ResizeVertical
    } else if h.y == 0.0 {
        egui::CursorIcon::ResizeHorizontal
    } else if h.x == h.y {
        egui::CursorIcon::ResizeNwSe
    } else {
        egui::CursorIcon::ResizeNeSw
    }
}
pub(super) fn paint_handles(
    painter: &egui::Painter,
    item: &SceneItem,
    screen: impl Fn(Rect) -> Rect,
    dim: (Option<u16>, Option<u16>),
    selection_color: Color32,
) {
    for h in handles(dim) {
        let r = Rect::from_center_size(handle_pos(screen(item.rect), h), Vec2::splat(6.0));
        painter.rect_filled(r, 0.0, Color32::WHITE);
        painter.rect_stroke(
            r,
            0.0,
            Stroke::new(1.0, selection_color),
            egui::StrokeKind::Inside,
        );
    }
}
pub(super) fn resize_edits(
    project: &EditorProject,
    item: &SceneItem,
    handle: Vec2,
    delta: Vec2,
    mods: Modifiers,
) -> Vec<CanvasEdit> {
    let proposed = resize_edits_unchecked(project, item, handle, delta, mods);
    if !proposed.is_empty() && valid_canvas_edits(project, &proposed) {
        return proposed;
    }

    // Shared dimensions affect every placement of this object, including
    // placements outside the active mask. Search along the requested gesture
    // so aspect ratio and center resizing remain intact at the boundary.
    let (w, h) = dimensions(project, item);
    let Some(target) = item.target.clone() else {
        return vec![];
    };
    let Some((x, y)) = position(project, &target) else {
        return vec![];
    };
    let baseline = vec![
        CanvasEdit::Resize {
            object_id: item.object_id,
            width: w,
            height: h,
        },
        CanvasEdit::Move { target, x, y },
    ];
    if !valid_canvas_edits(project, &baseline) {
        return vec![];
    }
    let mut low = 0.0;
    let mut high = 1.0;
    let mut best = baseline;
    for _ in 0..16 {
        let fraction = (low + high) * 0.5;
        let candidate = resize_edits_unchecked(project, item, handle, delta * fraction, mods);
        if !candidate.is_empty() && valid_canvas_edits(project, &candidate) {
            low = fraction;
            best = candidate;
        } else {
            high = fraction;
        }
    }
    best
}

fn resize_edits_unchecked(
    project: &EditorProject,
    item: &SceneItem,
    handle: Vec2,
    delta: Vec2,
    mods: Modifiers,
) -> Vec<CanvasEdit> {
    let (w, h) = dimensions(project, item);
    let Some(target) = item.target.clone() else {
        return vec![];
    };
    let Some((x, y)) = position(project, &target) else {
        return vec![];
    };
    let old = egui::vec2(
        w.map_or(item.rect.width(), |v| v as f32),
        h.map_or(item.rect.height(), |v| v as f32),
    );
    let mut size = old + delta * handle * if mods.alt { 2.0 } else { 1.0 };
    if mods.shift && w.is_some() && h.is_some() {
        let ratio = old.x / old.y.max(1.0);
        if handle.x == 0.0 {
            size.x = size.y * ratio
        } else if handle.y == 0.0
            || ((size.x - old.x) / old.x.max(1.0)).abs()
                >= ((size.y - old.y) / old.y.max(1.0)).abs()
        {
            size.y = size.x / ratio.max(0.001)
        } else {
            size.x = size.y * ratio
        }
    }
    size = size
        .round()
        .clamp(Vec2::splat(1.0), Vec2::splat(u16::MAX as f32));
    let diff = size - old;
    let shift = if mods.alt {
        -diff * 0.5
    } else {
        egui::vec2(
            if handle.x < 0.0 { -diff.x } else { 0.0 },
            if handle.y < 0.0 { -diff.y } else { 0.0 },
        )
    };
    let nx = (x as f32 + shift.x).round();
    let ny = (y as f32 + shift.y).round();
    if nx < i16::MIN as f32 || nx > i16::MAX as f32 || ny < i16::MIN as f32 || ny > i16::MAX as f32
    {
        return vec![];
    }
    vec![
        CanvasEdit::Resize {
            object_id: item.object_id,
            width: w.map(|_| size.x as u16),
            height: h.map(|_| size.y as u16),
        },
        CanvasEdit::Move {
            target,
            x: nx as i16,
            y: ny as i16,
        },
    ]
}

pub(super) fn resized_rect(
    project: &EditorProject,
    item: &SceneItem,
    edits: &[CanvasEdit],
) -> Option<Rect> {
    let (old_x, old_y) = position(project, item.target.as_ref()?)?;
    let mut rect = item.rect;
    for edit in edits {
        match edit {
            CanvasEdit::Move { x, y, .. } => {
                rect = rect.translate(egui::vec2(
                    *x as f32 - old_x as f32,
                    *y as f32 - old_y as f32,
                ))
            }
            CanvasEdit::Resize { width, height, .. } => {
                let size = egui::vec2(
                    width.map_or(rect.width(), |w| w as f32),
                    height.map_or_else(
                        || {
                            if let Some(w) = width {
                                rect.height() * *w as f32 / rect.width().max(1.0)
                            } else {
                                rect.height()
                            }
                        },
                        |h| h as f32,
                    ),
                );
                rect = Rect::from_min_size(rect.min, size);
            }
            _ => {}
        }
    }
    Some(rect)
}
pub(super) fn snap_resize_delta(
    project: &EditorProject,
    item: &SceneItem,
    scene: &Scene,
    handle: Vec2,
    raw: Vec2,
    mods: Modifiers,
    zoom: f32,
    enabled: bool,
) -> (Vec2, Vec<(Pos2, Pos2)>) {
    if !enabled || mods.ctrl {
        return (raw, vec![]);
    }
    let Some(proposed) = resized_rect(
        project,
        item,
        &resize_edits(project, item, handle, raw, mods),
    ) else {
        return (raw, vec![]);
    };
    let point = handle_pos(proposed, handle);
    let parent_path = &item.path[..item.path.len().saturating_sub(1)];
    let references: Vec<Rect> = std::iter::once(item.parent_rect)
        .chain(
            scene
                .items
                .iter()
                .filter(|s| {
                    s.path.len() == item.path.len()
                        && s.path.starts_with(parent_path)
                        && s.target != item.target
                })
                .map(|s| s.rect),
        )
        .collect();
    let mut delta = raw;
    let mut guides = vec![];
    for horizontal in [true, false] {
        if (horizontal && handle.x == 0.0) || (!horizontal && handle.y == 0.0) {
            continue;
        }
        let coordinate = if horizontal { point.x } else { point.y };
        let mut best = 6.0 / zoom + 1.0;
        let mut correction = 0.0;
        let mut line = None;
        for reference in &references {
            for target in anchors(*reference, horizontal) {
                let distance = target - coordinate;
                if distance.abs() <= 6.0 / zoom && distance.abs() < best {
                    best = distance.abs();
                    correction = distance;
                    line = Some(if horizontal {
                        (
                            egui::pos2(target, proposed.top().min(reference.top())),
                            egui::pos2(target, proposed.bottom().max(reference.bottom())),
                        )
                    } else {
                        (
                            egui::pos2(proposed.left().min(reference.left()), target),
                            egui::pos2(proposed.right().max(reference.right()), target),
                        )
                    });
                }
            }
        }
        if horizontal {
            delta.x += correction
        } else {
            delta.y += correction
        }
        if let Some(line) = line {
            guides.push(line);
        }
    }
    if let Some(actual) = resized_rect(
        project,
        item,
        &resize_edits(project, item, handle, delta, mods),
    ) {
        let actual = handle_pos(actual, handle);
        guides.retain(|(a, b)| {
            if a.x == b.x {
                (actual.x - a.x).abs() <= 0.5
            } else {
                (actual.y - a.y).abs() <= 0.5
            }
        });
    }
    (delta, guides)
}

pub(super) fn anchors(r: Rect, horizontal: bool) -> [f32; 3] {
    if horizontal {
        [r.left(), r.center().x, r.right()]
    } else {
        [r.top(), r.center().y, r.bottom()]
    }
}
pub(super) fn snap_delta(
    items: &[SceneItem],
    scene: &Scene,
    delta: Vec2,
    zoom: f32,
    objects: bool,
    spacing: bool,
    axis: bool,
) -> (Vec2, Vec<(Pos2, Pos2)>) {
    let Some(group) = bounds(items) else {
        return (delta, vec![]);
    };
    let moved = group.translate(delta);
    let threshold = 6.0 / zoom;
    let mut result = delta;
    let mut guides = vec![];
    for horizontal in [true, false] {
        if axis && ((horizontal && delta.x == 0.0) || (!horizontal && delta.y == 0.0)) {
            continue;
        }
        let mut best = threshold + 1.0;
        let mut correction = 0.0;
        let mut best_guides = vec![];
        for item in items {
            let parent_path = &item.path[..item.path.len().saturating_sub(1)];
            let siblings: Vec<_> = scene
                .items
                .iter()
                .filter(|s| {
                    s.path.len() == item.path.len()
                        && s.path.starts_with(parent_path)
                        && !items.iter().any(|i| s.path.starts_with(&i.path))
                })
                .collect();
            if objects {
                for reference in
                    std::iter::once(item.parent_rect).chain(siblings.iter().map(|s| s.rect))
                {
                    for a in anchors(moved, horizontal) {
                        for b in anchors(reference, horizontal) {
                            let distance = b - a;
                            if distance.abs() < best && distance.abs() <= threshold {
                                best = distance.abs();
                                correction = distance;
                                best_guides = vec![if horizontal {
                                    (
                                        egui::pos2(b, moved.top().min(reference.top())),
                                        egui::pos2(b, moved.bottom().max(reference.bottom())),
                                    )
                                } else {
                                    (
                                        egui::pos2(moved.left().min(reference.left()), b),
                                        egui::pos2(moved.right().max(reference.right()), b),
                                    )
                                }];
                            }
                        }
                    }
                }
            }
            if spacing {
                let start = |r: Rect| if horizontal { r.left() } else { r.top() };
                let end = |r: Rect| if horizontal { r.right() } else { r.bottom() };
                let overlaps = |r: Rect| {
                    if horizontal {
                        r.bottom() >= moved.top() - threshold
                            && r.top() <= moved.bottom() + threshold
                    } else {
                        r.right() >= moved.left() - threshold
                            && r.left() <= moved.right() + threshold
                    }
                };
                let mut neighbors: Vec<_> = siblings
                    .iter()
                    .copied()
                    .filter(|s| overlaps(s.rect))
                    .collect();
                neighbors.sort_by(|a, b| start(a.rect).total_cmp(&start(b.rect)));
                for pair in neighbors.windows(2) {
                    let (a, b) = (pair[0], pair[1]);
                    if end(a.rect) > start(b.rect) {
                        continue;
                    }
                    let candidates = [
                        (
                            (end(a.rect) + start(b.rect) - start(moved) - end(moved)) * 0.5,
                            0,
                        ),
                        (
                            end(b.rect) + (start(b.rect) - end(a.rect)) - start(moved),
                            1,
                        ),
                        (
                            start(a.rect) - (start(b.rect) - end(a.rect)) - end(moved),
                            2,
                        ),
                    ];
                    for (distance, mode) in candidates {
                        if distance.abs() >= best || distance.abs() > threshold {
                            continue;
                        }
                        let r = moved.translate(if horizontal {
                            egui::vec2(distance, 0.0)
                        } else {
                            egui::vec2(0.0, distance)
                        });
                        if mode == 0 && (end(a.rect) > start(r) || end(r) > start(b.rect)) {
                            continue;
                        }
                        best = distance.abs();
                        correction = distance;
                        let ordered = match mode {
                            0 => [a.rect, r, b.rect],
                            1 => [a.rect, b.rect, r],
                            _ => [r, a.rect, b.rect],
                        };
                        best_guides = ordered
                            .windows(2)
                            .map(|pair| {
                                if horizontal {
                                    (
                                        egui::pos2(pair[0].right(), r.center().y),
                                        egui::pos2(pair[1].left(), r.center().y),
                                    )
                                } else {
                                    (
                                        egui::pos2(r.center().x, pair[0].bottom()),
                                        egui::pos2(r.center().x, pair[1].top()),
                                    )
                                }
                            })
                            .collect();
                    }
                }
            }
        }
        if horizontal {
            result.x += correction
        } else {
            result.y += correction
        }
        guides.extend(best_guides);
    }
    (result.round(), guides)
}
pub(super) fn label(p: &egui::Painter, pos: Pos2, text: String, color: Color32) {
    let galley = p.layout_no_wrap(text, egui::FontId::monospace(14.0), color);
    let mut rect = Rect::from_min_size(
        pos + egui::vec2(3.0, -3.0) - egui::vec2(0.0, galley.size().y),
        galley.size(),
    )
    .expand2(egui::vec2(4.0, 2.0));
    let clip = p.clip_rect();
    let shift = egui::vec2(
        (clip.left() - rect.left()).max(0.0) - (rect.right() - clip.right()).max(0.0),
        (clip.top() - rect.top()).max(0.0) - (rect.bottom() - clip.bottom()).max(0.0),
    );
    rect = rect.translate(shift);
    p.rect_filled(rect, 3.0, Color32::from_rgba_unmultiplied(20, 23, 27, 240));
    p.galley(rect.min + egui::vec2(4.0, 2.0), galley, color);
}
pub(super) fn measure(
    p: &egui::Painter,
    object: Rect,
    reference: Rect,
    zoom: f32,
    guide_color: Color32,
) {
    let containing = reference.contains_rect(object);
    let segments = if containing {
        vec![
            (
                egui::pos2(reference.left(), object.center().y),
                object.left_center(),
                (object.left() - reference.left()) / zoom,
            ),
            (
                object.right_center(),
                egui::pos2(reference.right(), object.center().y),
                (reference.right() - object.right()) / zoom,
            ),
            (
                egui::pos2(object.center().x, reference.top()),
                object.center_top(),
                (object.top() - reference.top()) / zoom,
            ),
            (
                object.center_bottom(),
                egui::pos2(object.center().x, reference.bottom()),
                (reference.bottom() - object.bottom()) / zoom,
            ),
        ]
    } else {
        let (left, right) = if reference.right() <= object.left() {
            (reference.right(), object.left())
        } else if object.right() <= reference.left() {
            (object.right(), reference.left())
        } else {
            (reference.left(), object.left())
        };
        let (top, bottom) = if reference.bottom() <= object.top() {
            (reference.bottom(), object.top())
        } else if object.bottom() <= reference.top() {
            (object.bottom(), reference.top())
        } else {
            (reference.top(), object.top())
        };
        vec![
            (
                egui::pos2(left, object.center().y),
                egui::pos2(right, object.center().y),
                (right - left) / zoom,
            ),
            (
                egui::pos2(object.center().x, top),
                egui::pos2(object.center().x, bottom),
                (bottom - top) / zoom,
            ),
        ]
    };
    for (a, b, distance) in segments {
        p.line_segment([a, b], Stroke::new(1.0, guide_color));
        let tick = if a.y == b.y {
            egui::vec2(0.0, 3.0)
        } else {
            egui::vec2(3.0, 0.0)
        };
        p.line_segment([a - tick, a + tick], Stroke::new(1.0, guide_color));
        p.line_segment([b - tick, b + tick], Stroke::new(1.0, guide_color));
        label(p, a + (b - a) * 0.5, format!("{distance:.0}"), guide_color);
    }
}

pub(super) fn parent_measure(
    p: &egui::Painter,
    object: Rect,
    parent: Rect,
    zoom: f32,
    guide_color: Color32,
    selection_color: Color32,
) {
    for (a, b, value) in [
        (
            egui::pos2(parent.left(), object.center().y),
            object.left_center(),
            object.left() - parent.left(),
        ),
        (
            object.right_center(),
            egui::pos2(parent.right(), object.center().y),
            parent.right() - object.right(),
        ),
        (
            egui::pos2(object.center().x, parent.top()),
            object.center_top(),
            object.top() - parent.top(),
        ),
        (
            object.center_bottom(),
            egui::pos2(object.center().x, parent.bottom()),
            parent.bottom() - object.bottom(),
        ),
    ] {
        p.line_segment([a, b], Stroke::new(1.0, guide_color));
        label(
            p,
            a + (b - a) * 0.5,
            format!("{:.0}", value / zoom),
            guide_color,
        );
    }
    label(
        p,
        object.left_bottom() + egui::vec2(0.0, 17.0),
        format!(
            "X {:.0}  Y {:.0} · {:.0} × {:.0}",
            (object.left() - parent.left()) / zoom,
            (object.top() - parent.top()) / zoom,
            object.width() / zoom,
            object.height() / zoom
        ),
        selection_color,
    );
}
