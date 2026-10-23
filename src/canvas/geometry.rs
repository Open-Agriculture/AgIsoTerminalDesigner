//! Pool-coordinate geometry shared by canvas rendering, hit testing, and edits.

use super::edits::PlacementTarget;
use crate::{
    components::{MaterializedProject, MaterializedSource},
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
    /// Object occurrence chosen by the user. For an ObjectPointer this is the
    /// pointer itself, while `object_id` remains the resolved render target.
    pub selection_id: u16,
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
        None,
    );
    let keys = super::softkeys::build(project, &materialized.pool, root, page);
    for slot in keys.slots {
        if let super::softkeys::SlotContent::Key { id, selection_id } = slot.content {
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
                Some(selection_id),
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
        selection_id_override: Option<u16>,
    ) {
        let Some(source_object) = object_by_id(self.materialized, object_id) else {
            return;
        };
        // An ObjectPointer paints its referred object at the pointer placement.
        // Keep that occurrence identity separately from the geometry used to
        // render and edit the target definition.
        let selection_id = selection_id_override.unwrap_or_else(|| source_object.id().value());
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
                let inset =
                    crate::button_geometry::object_face_inset(rendered, self.project.vt_version);
                let face = raw_outer.shrink(inset);
                (face.min, rect.shrink(inset), face.intersect(clip_rect))
            }
            Object::Container(_) => (origin, rect, raw_outer.intersect(clip_rect)),
            Object::WorkingSet(_)
            | Object::AuxiliaryFunctionType1(_)
            | Object::AuxiliaryInputType1(_)
            | Object::AuxiliaryFunctionType2(_)
            | Object::AuxiliaryInputType2(_) => {
                let (width, height) = self.project.soft_key_size();
                let designator_rect =
                    egui::Rect::from_min_size(origin, egui::vec2(width as f32, height as f32));
                (
                    origin,
                    designator_rect,
                    designator_rect.intersect(clip_rect),
                )
            }
            Object::Key(_) => (origin, clip_rect, clip_rect),
            _ if is_root => (origin, rect, raw_outer.intersect(clip_rect)),
            _ => (clip_rect.min, clip_rect, clip_rect),
        };
        self.items.push(SceneItem {
            path: path.clone(),
            object_id,
            selection_id,
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
                None,
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
        if matches!(
            object,
            Object::Key(_)
                | Object::WorkingSet(_)
                | Object::AuxiliaryFunctionType1(_)
                | Object::AuxiliaryInputType1(_)
                | Object::AuxiliaryFunctionType2(_)
                | Object::AuxiliaryInputType2(_)
        ) {
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
        | Object::AuxiliaryFunctionType1(_)
        | Object::AuxiliaryInputType1(_)
        | Object::AuxiliaryFunctionType2(_)
        | Object::AuxiliaryInputType2(_) => object_refs(object),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ag_iso_stack::object_pool::{object_attributes::Point, ObjectPool, ObjectRef, ObjectType};

    #[test]
    fn pointer_scene_item_keeps_pointer_selection_and_target_geometry() {
        let mut pool = ObjectPool::default();
        for (id, object_type) in [
            (2, ObjectType::DataMask),
            (100, ObjectType::ObjectPointer),
            (101, ObjectType::OutputRectangle),
        ] {
            let mut object = crate::default_object(object_type);
            object.mut_id().set_value(id).unwrap();
            pool.add(object);
        }
        if let Some(Object::ObjectPointer(pointer)) =
            pool.object_mut_by_id(ObjectId::new(100).unwrap())
        {
            pointer.value = ObjectId::new(101).unwrap().into();
        }
        if let Some(Object::DataMask(mask)) = pool.object_mut_by_id(ObjectId::new(2).unwrap()) {
            mask.object_refs.push(ObjectRef {
                id: ObjectId::new(100).unwrap(),
                offset: Point { x: 30, y: 40 },
            });
        }

        let project = EditorProject::from(pool);
        let materialized = project.materialize_shared().unwrap();
        let scene = build_page(&project, &materialized, 2, 0);
        let pointer_occurrence = scene.items.iter().find(|item| item.path == [0]).unwrap();

        assert_eq!(pointer_occurrence.selection_id, 100);
        assert_eq!(pointer_occurrence.object_id, 101);
        assert_eq!(
            pointer_occurrence.target,
            Some(PlacementTarget::Primitive {
                parent_id: 2,
                child_index: 0,
            })
        );
        assert_eq!(pointer_occurrence.rect.min, egui::pos2(30.0, 40.0));
        assert!(!scene.items.iter().any(|item| item.path == [0, 0]));
    }

    #[test]
    fn soft_key_pointer_keeps_pointer_selection_identity() {
        let mut pool = ObjectPool::default();
        for (id, object_type) in [
            (2, ObjectType::DataMask),
            (3, ObjectType::SoftKeyMask),
            (100, ObjectType::ObjectPointer),
            (101, ObjectType::Key),
        ] {
            let mut object = crate::default_object(object_type);
            object.mut_id().set_value(id).unwrap();
            pool.add(object);
        }
        if let Some(Object::ObjectPointer(pointer)) =
            pool.object_mut_by_id(ObjectId::new(100).unwrap())
        {
            pointer.value = ObjectId::new(101).unwrap().into();
        }
        if let Some(Object::DataMask(mask)) = pool.object_mut_by_id(ObjectId::new(2).unwrap()) {
            mask.soft_key_mask = ObjectId::new(3).unwrap().into();
        }
        if let Some(Object::SoftKeyMask(mask)) = pool.object_mut_by_id(ObjectId::new(3).unwrap()) {
            mask.objects.push(ObjectId::new(100).unwrap());
        }

        let project = EditorProject::from(pool);
        let materialized = project.materialize_shared().unwrap();
        let scene = build_page(&project, &materialized, 2, 0);
        let key_occurrence = scene
            .items
            .iter()
            .find(|item| item.path == [super::super::softkeys::PATH_PREFIX, 0])
            .unwrap();

        assert_eq!(key_occurrence.selection_id, 100);
        assert_eq!(key_occurrence.object_id, 101);
    }

    #[test]
    fn designator_children_keep_their_canvas_origin_and_clip_to_soft_key_size() {
        for parent_type in [
            ObjectType::WorkingSet,
            ObjectType::AuxiliaryFunctionType1,
            ObjectType::AuxiliaryInputType1,
            ObjectType::AuxiliaryFunctionType2,
            ObjectType::AuxiliaryInputType2,
        ] {
            let mut pool = ObjectPool::default();
            for (id, object_type) in [
                (2, ObjectType::DataMask),
                (100, parent_type),
                (101, ObjectType::OutputRectangle),
            ] {
                let mut object = crate::default_object(object_type);
                object.mut_id().set_value(id).unwrap();
                pool.add(object);
            }
            if let Some(Object::DataMask(mask)) = pool.object_mut_by_id(ObjectId::new(2).unwrap()) {
                mask.object_refs.push(ObjectRef {
                    id: ObjectId::new(100).unwrap(),
                    offset: Point { x: 80, y: 60 },
                });
            }
            if let Some(parent) = pool.object_mut_by_id(ObjectId::new(100).unwrap()) {
                object_refs_mut(parent).unwrap().push(ObjectRef {
                    id: ObjectId::new(101).unwrap(),
                    offset: Point { x: 5, y: 6 },
                });
            }

            let mut project = EditorProject::from(pool);
            project.mask_size = 500;
            let soft_key_size = project.soft_key_size();
            let materialized = project.materialize_shared().unwrap();
            let scene = build_page(&project, &materialized, 2, 0);
            let designator = scene
                .items
                .iter()
                .find(|item| item.object_id == 100)
                .unwrap();
            let child = scene
                .items
                .iter()
                .find(|item| item.object_id == 101)
                .unwrap();
            let expected_parent = egui::Rect::from_min_size(
                egui::pos2(80.0, 60.0),
                egui::vec2(soft_key_size.0 as f32, soft_key_size.1 as f32),
            );
            assert_eq!(designator.rect, expected_parent);
            assert_eq!(child.rect.min, egui::pos2(85.0, 66.0));
            assert_eq!(child.parent_rect, expected_parent);
            assert_eq!(child.hit_rect, child.rect.intersect(expected_parent));
        }
    }

    fn object_refs_mut(
        object: &mut Object,
    ) -> Option<&mut Vec<ag_iso_stack::object_pool::ObjectRef>> {
        match object {
            Object::WorkingSet(value) => Some(&mut value.object_refs),
            Object::AuxiliaryFunctionType1(value) => Some(&mut value.object_refs),
            Object::AuxiliaryInputType1(value) => Some(&mut value.object_refs),
            Object::AuxiliaryFunctionType2(value) => Some(&mut value.object_refs),
            Object::AuxiliaryInputType2(value) => Some(&mut value.object_refs),
            _ => None,
        }
    }
}
