//! Pool-coordinate geometry shared by canvas rendering, hit testing, and edits.

use super::edits::PlacementTarget;
use crate::{
    components::{MaterializedProject, MaterializedSource},
    object_rendering::BUTTON_BORDER_INSET,
    operations::object_refs,
    EditorProject,
};
use ag_iso_stack::object_pool::{object::Object, ObjectId};
use eframe::egui;
use std::collections::HashSet;

#[derive(Clone, Debug)]
pub(crate) struct Scene {
    pub items: Vec<SceneItem>,
}

#[derive(Clone, Debug)]
pub(crate) struct SceneItem {
    /// Child-list indices from the requested root to this visible occurrence.
    pub path: Vec<usize>,
    /// The object whose visible geometry is rendered at this occurrence.
    pub object_id: u16,
    /// Unclipped, nominal bounds in pool coordinates.
    pub rect: egui::Rect,
    /// The nominal bounds of the parent viewport that clips this occurrence.
    pub parent_rect: egui::Rect,
    /// Exact editable occurrence; the requested root itself has no placement.
    pub target: Option<PlacementTarget>,
    /// Visible portion after all ancestor clipping. Use for pointer hit testing.
    pub hit_rect: egui::Rect,
}

pub(crate) fn build_page(
    project: &EditorProject,
    materialized: &MaterializedProject,
    root: u16,
    page: usize,
) -> Scene {
    let Some(root_object) = object_by_id(materialized, root) else {
        return Scene { items: Vec::new() };
    };
    let root_bounds = egui::Rect::from_min_size(
        egui::Pos2::ZERO,
        egui::vec2(project.mask_size as f32, project.mask_size as f32),
    );
    let mut builder = SceneBuilder {
        project,
        materialized,
        items: Vec::new(),
        ancestry: HashSet::new(),
    };
    let root_id = root_object.id().value();
    builder.ancestry.insert(root_id);
    builder.push_object(
        root_id,
        root_bounds.min,
        root_bounds,
        root_bounds,
        Vec::new(),
        None,
        true,
    );
    let keys = super::softkeys::build(project, &materialized.pool, root, page);
    for slot in keys.slots {
        if let super::softkeys::SlotContent::Key(id) = slot.content {
            builder.ancestry.clear();
            builder.ancestry.insert(id);
            builder.push_object(
                id,
                slot.rect.min,
                slot.rect,
                slot.rect,
                vec![super::softkeys::PATH_PREFIX, slot.index],
                None,
                false,
            );
        }
    }
    Scene {
        items: builder.items,
    }
}

struct SceneBuilder<'a> {
    project: &'a EditorProject,
    materialized: &'a MaterializedProject,
    items: Vec<SceneItem>,
    ancestry: HashSet<u16>,
}

impl SceneBuilder<'_> {
    #[allow(clippy::too_many_arguments)]
    fn push_object(
        &mut self,
        object_id: u16,
        origin: egui::Pos2,
        parent_rect: egui::Rect,
        clip_rect: egui::Rect,
        path: Vec<usize>,
        target: Option<PlacementTarget>,
        is_root: bool,
    ) {
        let Some(source_object) = object_by_id(self.materialized, object_id) else {
            return;
        };
        // An ObjectPointer paints its referred object at the pointer placement.
        // Its visible hit/selection identity is therefore the referred object,
        // while `target` continues to identify the pointer's child-list entry.
        let Some(rendered_id) = self.resolve_pointer_id(source_object) else {
            return;
        };
        let Some(rendered) = object_by_id(self.materialized, rendered_id) else {
            return;
        };
        if is_hidden(rendered) {
            return;
        }
        let object_id = rendered_id;
        let size = if is_root {
            // The mask canvas is the editor's authoritative viewport, including
            // masks whose visible content does not reach the right/bottom edge.
            (self.project.mask_size, self.project.mask_size)
        } else {
            self.size_of(rendered, &mut HashSet::new())
        };
        let rect = egui::Rect::from_min_size(origin, egui::vec2(size.0 as f32, size.1 as f32));
        let hit_rect = rect.intersect(clip_rect);

        // Copy child placement data while borrowing the materialized object;
        // recursive traversal below mutates the builder's ancestry and items.
        let children = visible_children(rendered)
            .map(|refs| {
                refs.iter()
                    .enumerate()
                    .map(|(index, reference)| {
                        (
                            index,
                            reference.id.value(),
                            reference.offset.x,
                            reference.offset.y,
                        )
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        let raw_outer = egui::Rect::from_min_max(
            origin,
            egui::pos2(
                rect.max.x.min(clip_rect.max.x).max(origin.x),
                rect.max.y.min(clip_rect.max.y).max(origin.y),
            ),
        );
        let (child_origin, child_parent_rect, child_clip_rect) = match rendered {
            Object::Button(_) => {
                let face = raw_outer.shrink(BUTTON_BORDER_INSET);
                (
                    face.min,
                    rect.shrink(BUTTON_BORDER_INSET),
                    face.intersect(clip_rect),
                )
            }
            Object::Container(_) => (origin, rect, raw_outer.intersect(clip_rect)),
            Object::Key(_) | Object::AuxiliaryFunctionType2(_) | Object::AuxiliaryInputType2(_) => {
                (origin, clip_rect, clip_rect)
            }
            _ if is_root => (origin, rect, raw_outer.intersect(clip_rect)),
            _ => (clip_rect.min, clip_rect, clip_rect),
        };
        self.items.push(SceneItem {
            path: path.clone(),
            object_id,
            rect,
            parent_rect,
            target,
            hit_rect,
        });

        // A materialized component root is the instance's movable unit. Its
        // generated descendants have no independent canvas placement targets.
        if matches!(
            self.materialized.source_map.by_object_id.get(&object_id),
            Some(MaterializedSource::ComponentNode { .. })
        ) {
            return;
        }
        if children.is_empty() {
            return;
        }
        for (index, child_id, x, y) in children {
            let Some(child) = object_by_id(self.materialized, child_id) else {
                continue;
            };
            let mut child_path = path.clone();
            child_path.push(index);
            let placement = self.placement_target(rendered_id, index, child_id);
            let Some(traversal_id) = self.resolve_pointer_id(child) else {
                continue;
            };
            // Guard cycles along this occurrence only, allowing repeated shared
            // references to appear independently elsewhere in the scene.
            if !self.ancestry.insert(traversal_id) {
                continue;
            }
            let child_origin = egui::pos2(child_origin.x + x as f32, child_origin.y + y as f32);
            self.push_object(
                child_id,
                child_origin,
                child_parent_rect,
                child_clip_rect,
                child_path,
                placement,
                false,
            );
            self.ancestry.remove(&traversal_id);
        }
    }

    fn placement_target(
        &self,
        parent_id: u16,
        materialized_index: usize,
        child_id: u16,
    ) -> Option<PlacementTarget> {
        if let Some(MaterializedSource::ComponentNode { instance_id, .. }) =
            self.materialized.source_map.by_object_id.get(&child_id)
        {
            // Only the virtual root appears as a child of a canonical parent;
            // generated descendants are collapsed into this instance item.
            return Some(PlacementTarget::Component {
                instance_id: *instance_id,
            });
        }

        let inserted_before = self
            .materialized
            .pool
            .object_by_id(ObjectId::new(parent_id).ok()?)
            .and_then(object_refs)
            .unwrap_or_default()
            .iter()
            .take(materialized_index)
            .filter(|reference| {
                matches!(
                    self.materialized
                        .source_map
                        .by_object_id
                        .get(&reference.id.value()),
                    Some(MaterializedSource::ComponentNode { .. })
                )
            })
            .count();
        let canonical_index = materialized_index.saturating_sub(inserted_before);
        // A child of a compiled component node is not an independently editable
        // occurrence because the component root is represented as one item.
        if matches!(
            self.materialized.source_map.by_object_id.get(&parent_id),
            Some(MaterializedSource::ComponentNode { .. })
        ) {
            return None;
        }
        Some(PlacementTarget::Primitive {
            parent_id,
            child_index: canonical_index,
        })
    }

    fn size_of(&self, object: &Object, visiting: &mut HashSet<u16>) -> (u16, u16) {
        if matches!(object, Object::Key(_)) {
            return self.project.soft_key_size();
        }
        if let Some(size) = object.as_sized_object() {
            return (size.width(), size.height());
        }
        let id = object.id().value();
        if !visiting.insert(id) {
            return (0, 0);
        }
        let mut width = 0_i32;
        let mut height = 0_i32;
        if let Some(children) = visible_children(object) {
            for child_ref in children {
                let Some(child) = self.materialized.pool.object_by_id(child_ref.id) else {
                    continue;
                };
                let Some(child_id) = self.resolve_pointer_id(child) else {
                    continue;
                };
                let Some(child) = object_by_id(self.materialized, child_id) else {
                    continue;
                };
                let (child_width, child_height) = self.size_of(child, visiting);
                width = width.max(child_ref.offset.x as i32 + child_width as i32);
                height = height.max(child_ref.offset.y as i32 + child_height as i32);
            }
        }
        visiting.remove(&id);
        (
            width.clamp(0, u16::MAX as i32) as u16,
            height.clamp(0, u16::MAX as i32) as u16,
        )
    }

    /// Follow pointer aliases to the object the renderer ultimately paints.
    /// Malformed alias cycles and empty pointers produce no visible object.
    fn resolve_pointer_id(&self, object: &Object) -> Option<u16> {
        let mut current = object;
        let mut seen = HashSet::new();
        loop {
            let Object::ObjectPointer(pointer) = current else {
                return Some(current.id().value());
            };
            if !seen.insert(current.id().value()) {
                return None;
            }
            let Some(id) = pointer.value.0 else {
                return None;
            };
            let Some(target) = self.materialized.pool.object_by_id(id) else {
                // The renderer paints a missing-reference error at this alias.
                return Some(current.id().value());
            };
            current = target;
        }
    }
}

fn object_by_id(materialized: &MaterializedProject, id: u16) -> Option<&Object> {
    materialized.pool.object_by_id(ObjectId::new(id).ok()?)
}

fn is_hidden(object: &Object) -> bool {
    matches!(object, Object::Container(container) if container.hidden)
}

/// Only follow child lists that the preview renderer actually paints.
fn visible_children(object: &Object) -> Option<&[ag_iso_stack::object_pool::ObjectRef]> {
    match object {
        Object::WorkingSet(_)
        | Object::DataMask(_)
        | Object::AlarmMask(_)
        | Object::Container(_)
        | Object::Button(_)
        | Object::Key(_)
        | Object::AuxiliaryFunctionType2(_)
        | Object::AuxiliaryInputType2(_) => object_refs(object),
        _ => None,
    }
}
