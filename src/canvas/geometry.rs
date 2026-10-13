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

/// Build visible pool-coordinate occurrences for a mask and its rendered children.
/// Component internals are represented by one item at their virtual root placement.
pub(crate) fn build(
    project: &EditorProject,
    materialized: &MaterializedProject,
    root: u16,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default_object;
    use ag_iso_stack::object_pool::{object::Object, ObjectPool, ObjectRef, ObjectType};

    fn object_with_id(object_type: ObjectType, id: u16) -> Object {
        let mut object = default_object(object_type);
        object.mut_id().set_value(id).unwrap();
        object
    }

    fn reference(id: u16, x: i16, y: i16) -> ObjectRef {
        ObjectRef {
            id: ObjectId::new(id).unwrap(),
            offset: ag_iso_stack::object_pool::object_attributes::Point { x, y },
        }
    }

    #[test]
    fn scene_uses_nested_origins_button_inset_and_hidden_container_visibility() {
        let mut pool = ObjectPool::default();
        let mut mask = object_with_id(ObjectType::DataMask, 1);
        let mut button = object_with_id(ObjectType::Button, 2);
        let mut rectangle = object_with_id(ObjectType::OutputRectangle, 3);
        let mut hidden = object_with_id(ObjectType::Container, 4);
        if let Object::DataMask(mask) = &mut mask {
            mask.object_refs.extend([
                reference(2, 10, 12),
                reference(4, 40, 40),
                reference(3, 120, 120),
            ]);
        }
        if let Object::Button(button) = &mut button {
            button.width = 30;
            button.height = 24;
            button.object_refs.push(reference(3, -2, 3));
        }
        if let Object::OutputRectangle(rect) = &mut rectangle {
            rect.width = 12;
            rect.height = 8;
        }
        if let Object::Container(container) = &mut hidden {
            container.width = 30;
            container.height = 30;
            container.hidden = true;
            container.object_refs.push(reference(3, 0, 0));
        }
        pool.add(mask);
        pool.add(button);
        pool.add(rectangle);
        pool.add(hidden);
        let mut project = EditorProject::from(pool);
        project.mask_size = 100;
        let materialized = project.materialize().unwrap();

        let scene = build(&project, &materialized, 1);
        assert_eq!(scene.items.len(), 4);
        assert_eq!(scene.items[1].path, vec![0]);
        assert_eq!(scene.items[2].path, vec![0, 0]);
        assert_eq!(scene.items[2].rect.min, egui::pos2(12.0, 19.0));
        assert_eq!(
            scene.items[2].parent_rect,
            egui::Rect::from_min_max(egui::pos2(14.0, 16.0), egui::pos2(36.0, 32.0),)
        );
        assert_eq!(scene.items[2].hit_rect.min, egui::pos2(14.0, 19.0));
        assert_eq!(
            scene.items[2].target,
            Some(PlacementTarget::Primitive {
                parent_id: 2,
                child_index: 0,
            })
        );
        assert_eq!(scene.items[3].path, vec![2]);
        assert!(scene.items[3].hit_rect.width() <= 0.0);
    }

    #[test]
    fn pointer_alias_keeps_the_pointer_placement_as_its_edit_target() {
        let mut pool = ObjectPool::default();
        let mut mask = object_with_id(ObjectType::DataMask, 1);
        let target = object_with_id(ObjectType::OutputRectangle, 2);
        let mut pointer = object_with_id(ObjectType::ObjectPointer, 3);
        let mut alias = object_with_id(ObjectType::ObjectPointer, 4);
        if let Object::DataMask(mask) = &mut mask {
            mask.object_refs.push(reference(3, 7, 9));
        }
        if let Object::ObjectPointer(pointer) = &mut pointer {
            pointer.value.0 = Some(ObjectId::new(4).unwrap());
        }
        if let Object::ObjectPointer(alias) = &mut alias {
            alias.value.0 = Some(ObjectId::new(2).unwrap());
        }
        pool.add(mask);
        pool.add(target);
        pool.add(pointer);
        pool.add(alias);
        let mut project = EditorProject::from(pool);
        project.mask_size = 100;
        let materialized = project.materialize().unwrap();

        let scene = build(&project, &materialized, 1);
        assert_eq!(scene.items.len(), 2);
        assert_eq!(scene.items[1].object_id, 2);
        assert_eq!(scene.items[1].rect.min, egui::pos2(7.0, 9.0));
        assert_eq!(
            scene.items[1].target,
            Some(PlacementTarget::Primitive {
                parent_id: 1,
                child_index: 0,
            })
        );
    }

    #[test]
    fn pointer_to_hidden_container_has_no_visible_scene_item() {
        let mut pool = ObjectPool::default();
        let mut mask = object_with_id(ObjectType::DataMask, 1);
        let mut hidden = object_with_id(ObjectType::Container, 2);
        let mut pointer = object_with_id(ObjectType::ObjectPointer, 3);
        if let Object::DataMask(mask) = &mut mask {
            mask.object_refs.push(reference(3, 7, 9));
        }
        if let Object::Container(container) = &mut hidden {
            container.width = 30;
            container.height = 20;
            container.hidden = true;
        }
        if let Object::ObjectPointer(pointer) = &mut pointer {
            pointer.value.0 = Some(ObjectId::new(2).unwrap());
        }
        pool.add(mask);
        pool.add(hidden);
        pool.add(pointer);
        let mut project = EditorProject::from(pool);
        project.mask_size = 100;
        let materialized = project.materialize().unwrap();

        let scene = build(&project, &materialized, 1);
        assert_eq!(scene.items.len(), 1);
    }

    #[test]
    fn cyclic_pointer_aliases_are_skipped() {
        let mut pool = ObjectPool::default();
        let mut mask = object_with_id(ObjectType::DataMask, 1);
        let mut first = object_with_id(ObjectType::ObjectPointer, 2);
        let mut second = object_with_id(ObjectType::ObjectPointer, 3);
        if let Object::DataMask(mask) = &mut mask {
            mask.object_refs.push(reference(2, 0, 0));
        }
        if let Object::ObjectPointer(first) = &mut first {
            first.value.0 = Some(ObjectId::new(3).unwrap());
        }
        if let Object::ObjectPointer(second) = &mut second {
            second.value.0 = Some(ObjectId::new(2).unwrap());
        }
        pool.add(mask);
        pool.add(first);
        pool.add(second);
        let project = EditorProject::new_pool();
        let materialized = MaterializedProject {
            pool,
            source_map: Default::default(),
        };

        assert_eq!(build(&project, &materialized, 1).items.len(), 1);
    }
}
