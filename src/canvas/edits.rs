//! Atomic edits made by the interactive object-pool canvas.

use crate::{
    components::{
        ComponentError, ComponentInstanceId, ComponentResult, ComponentSource, EditorParentRef,
    },
    editor_project::EditorProject,
    object_info::ObjectInfo,
    operations::{object_refs, ChildPlacement, ObjectTarget, Operation, OperationContext},
};
use ag_iso_stack::object_pool::vt_version::VtVersion;
use ag_iso_stack::object_pool::{ObjectId, ObjectPool};
use serde_json::json;
use std::collections::{BTreeMap, HashMap, HashSet};

/// Identifies an editable occurrence in the current project revision.
/// Recompute targets after structural edits; canonical child indices are not
/// persistent object identities and exclude materialized component roots.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum PlacementTarget {
    Primitive { parent_id: u16, child_index: usize },
    Component { instance_id: ComponentInstanceId },
}

/// An edit to a placed object or its dimensions.
/// Positions are signed parent-relative pool pixels. Dimensions belong to the
/// shared object definition; omitted dimensions remain unchanged.
#[derive(Clone, Debug, PartialEq)]
pub enum CanvasEdit {
    Move {
        target: PlacementTarget,
        x: i16,
        y: i16,
    },
    Resize {
        object_id: u16,
        width: Option<u16>,
        height: Option<u16>,
    },
    Remove {
        target: PlacementTarget,
    },
    Duplicate {
        target: PlacementTarget,
        x: i16,
        y: i16,
    },
}

/// Return the current local placement coordinates for a canvas target.
pub fn position(project: &EditorProject, target: &PlacementTarget) -> Option<(i16, i16)> {
    match target {
        PlacementTarget::Primitive {
            parent_id,
            child_index,
        } => child_placements(project.get_pool(), *parent_id)
            .ok()?
            .get(*child_index)
            .map(|placement| (placement.x, placement.y)),
        PlacementTarget::Component { instance_id } => project
            .component_source()
            .instances
            .iter()
            .find(|instance| instance.id == *instance_id)
            .and_then(|instance| {
                Some((
                    i16::try_from(instance.x).ok()?,
                    i16::try_from(instance.y).ok()?,
                ))
            }),
    }
}

/// Apply a canvas edit batch as one undoable, validated project change.
///
/// Primitive targets use their canonical parent and child index, so repeated
/// references to the same object remain independently editable. A successful
/// changed batch creates one history entry; no-op batches create none, and any
/// invalid edit leaves the project intact. Returned targets identify surviving
/// placements after the batch's removals have shifted their child indices.
/// Move and Duplicate supply selection targets; a Resize alone supplies none.
/// Primitive indices refer to the lists at the beginning of this batch.
pub fn apply(
    project: &mut EditorProject,
    description: &str,
    edits: &[CanvasEdit],
) -> Result<Vec<PlacementTarget>, String> {
    if edits.is_empty() {
        return Ok(Vec::new());
    }
    let mut seen_removals = HashSet::new();
    let mut removals = Vec::new();
    for edit in edits {
        if let CanvasEdit::Remove { target } = edit {
            if seen_removals.insert(target.clone()) {
                removals.push(target.clone());
            }
        }
    }
    let mut touched = Vec::new();
    let vt_version = project.vt_version;
    let id_ranges = project.object_id_ranges().clone();
    project
        .edit_source(description, |pool, source, _graphics, info| {
            // Placements refer to the batch's starting child indices. Apply
            // movement and duplication before removals, then remove primitive
            // occurrences from right to left within each canonical parent list.
            let mut ordered: Vec<_> = edits
                .iter()
                .filter(|e| !matches!(e, CanvasEdit::Remove { .. }))
                .collect();
            let mut removals: Vec<_> = edits
                .iter()
                .filter(|e| matches!(e, CanvasEdit::Remove { .. }))
                .collect();
            removals.sort_by(|left, right| compare_removals(left, right));
            let mut removal_seen = HashSet::new();
            for removal in removals {
                if let CanvasEdit::Remove { target } = removal {
                    if removal_seen.insert(target.clone()) {
                        ordered.push(removal);
                    }
                }
            }
            for edit in ordered {
                match edit {
                    CanvasEdit::Move { target, x, y } => {
                        move_target(pool, source, target, *x, *y, vt_version)?;
                        if !touched.contains(target) {
                            touched.push(target.clone());
                        }
                    }
                    CanvasEdit::Resize {
                        object_id,
                        width,
                        height,
                    } => resize_object(pool, *object_id, *width, *height)?,
                    CanvasEdit::Remove { target } => {
                        remove_target(pool, source, target, vt_version)?
                    }
                    CanvasEdit::Duplicate { target, x, y } => {
                        let duplicate = duplicate_target(
                            pool, source, info, target, *x, *y, vt_version, &id_ranges,
                        )?;
                        if let Some(index) = touched.iter().position(|existing| existing == target)
                        {
                            touched[index] = duplicate;
                        } else if !touched.contains(&duplicate) {
                            touched.push(duplicate);
                        }
                    }
                }
                if let CanvasEdit::Remove { target } = edit {
                    touched.retain(|existing| existing != target);
                }
            }
            Ok(())
        })
        .map_err(|e| e.to_string())?;
    normalize_touched_targets(&mut touched, &removals);
    Ok(touched)
}

fn normalize_touched_targets(targets: &mut [PlacementTarget], removals: &[PlacementTarget]) {
    for target in targets {
        if let PlacementTarget::Primitive {
            parent_id,
            child_index,
        } = target
        {
            let removed_before = removals
                .iter()
                .filter_map(|removed| match removed {
                    PlacementTarget::Primitive {
                        parent_id: removed_parent,
                        child_index: removed_index,
                    } if parent_id == removed_parent && removed_index < child_index => Some(()),
                    _ => None,
                })
                .count();
            *child_index = child_index.saturating_sub(removed_before);
        }
    }
}

fn move_target(
    pool: &mut ObjectPool,
    source: &mut ComponentSource,
    target: &PlacementTarget,
    x: i16,
    y: i16,
    vt_version: VtVersion,
) -> ComponentResult<()> {
    match target {
        PlacementTarget::Primitive {
            parent_id,
            child_index,
        } => {
            let placements = child_placements(pool, *parent_id)?;
            let placement = placements
                .get(*child_index)
                .cloned()
                .ok_or_else(invalid_placement)?;
            let mut updated = placements;
            updated[*child_index] = ChildPlacement { x, y, ..placement };
            set_child_placements(pool, *parent_id, updated, vt_version)
        }
        PlacementTarget::Component { instance_id } => {
            let instance = source
                .instances
                .iter_mut()
                .find(|i| i.id == *instance_id)
                .ok_or_else(invalid_placement)?;
            instance.x = i32::from(x);
            instance.y = i32::from(y);
            Ok(())
        }
    }
}

fn resize_object(
    pool: &mut ObjectPool,
    object_id: u16,
    width: Option<u16>,
    height: Option<u16>,
) -> ComponentResult<()> {
    let id = ObjectId::new(object_id).map_err(|e| {
        ComponentError::new("CANVAS_EDIT_INVALID", format!("Invalid object ID: {e:?}"))
    })?;
    if pool.object_by_id(id).is_none() {
        return Err(ComponentError::new(
            "CANVAS_EDIT_INVALID",
            format!("Object {object_id} is not an editable primitive object"),
        ));
    }
    for (property, value) in [
        (crate::object_properties::PropertyKey::Width, width),
        (crate::object_properties::PropertyKey::Height, height),
    ] {
        if let Some(value) = value {
            crate::object_properties::set_property(pool, id, property, json!(value))
                .map_err(|e| ComponentError::new("CANVAS_EDIT_INVALID", e.to_string()))?;
        }
    }
    Ok(())
}

fn remove_target(
    pool: &mut ObjectPool,
    source: &mut ComponentSource,
    target: &PlacementTarget,
    vt_version: VtVersion,
) -> ComponentResult<()> {
    match target {
        PlacementTarget::Primitive {
            parent_id,
            child_index,
        } => {
            let display_index = primitive_display_index(pool, source, *parent_id, *child_index)?;
            let mut placements = child_placements(pool, *parent_id)?;
            if *child_index >= placements.len() {
                return Err(invalid_placement());
            }
            placements.remove(*child_index);
            set_child_placements(pool, *parent_id, placements, vt_version)?;
            for instance in &mut source.instances {
                if instance.parent == EditorParentRef::Primitive(*parent_id)
                    && instance
                        .child_index
                        .is_some_and(|index| index > display_index)
                {
                    instance.child_index = instance.child_index.map(|index| index - 1);
                }
            }
            Ok(())
        }
        PlacementTarget::Component { instance_id } => {
            let index = source
                .instances
                .iter()
                .position(|i| i.id == *instance_id)
                .ok_or_else(invalid_placement)?;
            let removed = source.instances.remove(index);
            if let Some(child_index) = removed.child_index {
                for instance in &mut source.instances {
                    if instance.parent == removed.parent
                        && instance
                            .child_index
                            .is_some_and(|index| index > child_index)
                    {
                        instance.child_index = instance.child_index.map(|index| index - 1);
                    }
                }
            }
            let _ = pool;
            Ok(())
        }
    }
}

fn duplicate_target(
    pool: &mut ObjectPool,
    source: &mut ComponentSource,
    info: &mut HashMap<ObjectId, ObjectInfo>,
    target: &PlacementTarget,
    x: i16,
    y: i16,
    vt_version: VtVersion,
    id_ranges: &crate::ObjectIdRanges,
) -> ComponentResult<PlacementTarget> {
    match target {
        PlacementTarget::Component { instance_id } => {
            let mut instance = source
                .instances
                .iter()
                .find(|i| i.id == *instance_id)
                .cloned()
                .ok_or_else(invalid_placement)?;
            instance.id = ComponentInstanceId::default();
            instance.x = i32::from(x);
            instance.y = i32::from(y);
            instance.child_index = Some(insertion_index(pool, source, instance.parent.clone()));
            instance.compiled_ids.clear();
            let result = PlacementTarget::Component {
                instance_id: instance.id,
            };
            source.instances.push(instance);
            Ok(result)
        }
        PlacementTarget::Primitive {
            parent_id,
            child_index,
        } => {
            let placements = child_placements(pool, *parent_id)?;
            let root = placements
                .get(*child_index)
                .ok_or_else(invalid_placement)?
                .clone();
            let root_id = target_id(&root.child).ok_or_else(invalid_placement)?;
            let mut graph_ids = Vec::new();
            collect_positioned_subtree(pool, root_id, &mut HashSet::new(), &mut graph_ids)?;
            let mut copy_pool = ObjectPool::new();
            let mut copied_info = HashMap::new();
            for old in &graph_ids {
                let old_id = object_id(*old)?;
                let object = pool
                    .object_by_id(old_id)
                    .cloned()
                    .ok_or_else(invalid_placement)?;
                copy_pool.add(object);
                if let Some(metadata) = info.get(&old_id) {
                    copied_info.insert(old_id, metadata.clone());
                }
            }

            let reserved = reserved_ids(pool, source);
            let mut mapping = BTreeMap::new();
            let mut context = OperationContext::default();
            let mut operation_info = HashMap::new();
            for old in &graph_ids {
                let old_object = pool
                    .object_by_id(object_id(*old)?)
                    .ok_or_else(invalid_placement)?;
                let object_type = old_object.object_type();
                let range = id_ranges.range_for(object_type);
                let new = (range.start..=range.end)
                    .find(|id| {
                        !reserved.contains(id) && !mapping.values().any(|mapped| mapped == id)
                    })
                    .ok_or_else(|| {
                        ComponentError::new(
                            "CANVAS_ID_RANGE_EXHAUSTED",
                            format!(
                                "No free {object_type:?} ID remains in configured range {range}"
                            ),
                        )
                    })?;
                Operation::ChangeObjectId {
                    old_id: *old,
                    new_id: new,
                }
                .apply(&mut copy_pool, &mut operation_info, &mut context)
                .map_err(|e| ComponentError::new("CANVAS_DUPLICATE_FAILED", e.to_string()))?;
                mapping.insert(*old, new);
            }
            let new_root = mapping[&root_id];
            for (old_id, new_id) in &mapping {
                let old_object_id = object_id(*old_id)?;
                let new_object_id = object_id(*new_id)?;
                let copied_object = copy_pool
                    .object_by_id(new_object_id)
                    .ok_or_else(invalid_placement)?;
                let mut metadata = ObjectInfo::new(copied_object);
                // The duplicate receives an automatically allocated identity,
                // regardless of whether its source ID was manually assigned.
                metadata.id_overridden = false;
                if let Some(original) = copied_info.get(&old_object_id) {
                    metadata.description = original.description.clone();
                    metadata.tags = original.tags.clone();
                    metadata.allowed_references = original
                        .allowed_references
                        .iter()
                        .map(|id| {
                            mapping
                                .get(&id.value())
                                .map_or(*id, |mapped| ObjectId::new(*mapped).unwrap())
                        })
                        .collect();
                    if let Some(name) = &original.name {
                        metadata.name = Some(unique_copy_name(name, info));
                    }
                }
                info.insert(new_object_id, metadata);
            }
            for object in copy_pool.objects().iter().cloned() {
                pool.add(object);
            }
            for instance in source.instances.clone() {
                let EditorParentRef::Primitive(old_parent) = instance.parent;
                if let Some(new_parent) = mapping.get(&old_parent) {
                    let mut copy = instance;
                    copy.id = ComponentInstanceId::default();
                    copy.parent = EditorParentRef::Primitive(*new_parent);
                    copy.compiled_ids.clear();
                    source.instances.push(copy);
                }
            }
            let mut updated = placements;
            updated.push(ChildPlacement {
                child: ObjectTarget::id(new_root),
                x,
                y,
            });
            set_child_placements(pool, *parent_id, updated, vt_version)?;
            Ok(PlacementTarget::Primitive {
                parent_id: *parent_id,
                child_index: child_placements(pool, *parent_id)?.len() - 1,
            })
        }
    }
}

fn collect_positioned_subtree(
    pool: &ObjectPool,
    id: u16,
    visited: &mut HashSet<u16>,
    ordered: &mut Vec<u16>,
) -> ComponentResult<()> {
    if !visited.insert(id) {
        return Ok(());
    }
    let object = pool.object_by_id(object_id(id)?).ok_or_else(|| {
        ComponentError::new("CANVAS_EDIT_INVALID", format!("Missing object {id}"))
    })?;
    ordered.push(id);
    for reference in object_refs(object).unwrap_or_default() {
        collect_positioned_subtree(pool, reference.id.value(), visited, ordered)?;
    }
    Ok(())
}

fn child_placements(pool: &ObjectPool, parent_id: u16) -> ComponentResult<Vec<ChildPlacement>> {
    let object = pool.object_by_id(object_id(parent_id)?).ok_or_else(|| {
        ComponentError::new("CANVAS_EDIT_INVALID", format!("Missing parent {parent_id}"))
    })?;
    object_refs(object)
        .map(|refs| {
            refs.iter()
                .map(|reference| ChildPlacement {
                    child: ObjectTarget::id(reference.id.value()),
                    x: reference.offset.x,
                    y: reference.offset.y,
                })
                .collect()
        })
        .ok_or_else(|| {
            ComponentError::new(
                "CANVAS_EDIT_INVALID",
                "Parent does not contain positioned children",
            )
        })
}

fn set_child_placements(
    pool: &mut ObjectPool,
    parent_id: u16,
    children: Vec<ChildPlacement>,
    vt_version: VtVersion,
) -> ComponentResult<()> {
    let mut context = OperationContext::default();
    context.vt_version = vt_version;
    Operation::SetChildren {
        parent: ObjectTarget::id(parent_id),
        children,
    }
    .apply(pool, &mut HashMap::new(), &mut context)
    .map(|_| ())
    .map_err(|e| ComponentError::new("CANVAS_EDIT_INVALID", e.to_string()))
}

fn target_id(target: &ObjectTarget) -> Option<u16> {
    match target {
        ObjectTarget::Id { id } => Some(*id),
        ObjectTarget::Name { .. } => None,
    }
}

fn object_id(value: u16) -> ComponentResult<ObjectId> {
    ObjectId::new(value).map_err(|e| {
        ComponentError::new("CANVAS_EDIT_INVALID", format!("Invalid object ID: {e:?}"))
    })
}

fn reserved_ids(pool: &ObjectPool, source: &ComponentSource) -> HashSet<u16> {
    pool.objects()
        .iter()
        .map(|object| object.id().value())
        .chain(
            source
                .instances
                .iter()
                .flat_map(|instance| instance.compiled_ids.values().copied()),
        )
        .collect()
}

fn unique_copy_name(name: &str, info: &HashMap<ObjectId, ObjectInfo>) -> String {
    let taken: HashSet<_> = info
        .values()
        .filter_map(|metadata| metadata.name.as_deref())
        .collect();
    let base = format!("{name} copy");
    if !taken.contains(base.as_str()) {
        return base;
    }
    for suffix in 2u32.. {
        let candidate = format!("{base} {suffix}");
        if !taken.contains(candidate.as_str()) {
            return candidate;
        }
    }
    unreachable!("the name suffix space is effectively unbounded")
}

fn insertion_index(pool: &ObjectPool, source: &ComponentSource, parent: EditorParentRef) -> usize {
    let EditorParentRef::Primitive(parent_id) = parent;
    let primitive = object_id(parent_id)
        .ok()
        .and_then(|id| pool.object_by_id(id))
        .and_then(object_refs)
        .map_or(0, |refs| refs.len());
    primitive
        + source
            .instances
            .iter()
            .filter(|i| i.parent == EditorParentRef::Primitive(parent_id))
            .count()
}

fn primitive_display_index(
    pool: &ObjectPool,
    source: &ComponentSource,
    parent_id: u16,
    child_index: usize,
) -> ComponentResult<usize> {
    let children = child_placements(pool, parent_id)?;
    if child_index >= children.len() {
        return Err(invalid_placement());
    }
    // Mirror materialization: component roots are inserted into the canonical
    // child sequence in display-list order, so their indices count toward the
    // removal position of a later primitive occurrence.
    let mut display: Vec<Option<usize>> = (0..children.len()).map(Some).collect();
    let mut instances: Vec<_> = source
        .instances
        .iter()
        .filter(|instance| instance.parent == EditorParentRef::Primitive(parent_id))
        .collect();
    instances.sort_by_key(|instance| (instance.child_index.unwrap_or(usize::MAX), instance.id));
    for instance in instances {
        let index = instance
            .child_index
            .unwrap_or(display.len())
            .min(display.len());
        display.insert(index, None);
    }
    display
        .iter()
        .position(|primitive| *primitive == Some(child_index))
        .ok_or_else(invalid_placement)
}

fn compare_removals(left: &CanvasEdit, right: &CanvasEdit) -> std::cmp::Ordering {
    use std::cmp::Ordering;
    match (left, right) {
        (
            CanvasEdit::Remove {
                target:
                    PlacementTarget::Primitive {
                        parent_id: lp,
                        child_index: li,
                    },
            },
            CanvasEdit::Remove {
                target:
                    PlacementTarget::Primitive {
                        parent_id: rp,
                        child_index: ri,
                    },
            },
        ) => lp.cmp(rp).then_with(|| ri.cmp(li)),
        (
            CanvasEdit::Remove {
                target: PlacementTarget::Primitive { .. },
            },
            CanvasEdit::Remove {
                target: PlacementTarget::Component { .. },
            },
        ) => Ordering::Less,
        (
            CanvasEdit::Remove {
                target: PlacementTarget::Component { .. },
            },
            CanvasEdit::Remove {
                target: PlacementTarget::Primitive { .. },
            },
        ) => Ordering::Greater,
        (
            CanvasEdit::Remove {
                target: PlacementTarget::Component { instance_id: left },
            },
            CanvasEdit::Remove {
                target: PlacementTarget::Component { instance_id: right },
            },
        ) => left.cmp(right),
        _ => Ordering::Equal,
    }
}

fn invalid_placement() -> ComponentError {
    ComponentError::new("CANVAS_EDIT_INVALID", "Placement no longer exists")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{ComponentDefinition, ComponentId, ComponentNodeId, ComponentService};
    use crate::{default_object, EditorProject};
    use ag_iso_stack::object_pool::{object::Object, ObjectType};

    fn object_with_id(object_type: ObjectType, id: u16) -> Object {
        let mut object = default_object(object_type);
        object.mut_id().set_value(id).unwrap();
        object
    }

    fn project(pool: ObjectPool) -> EditorProject {
        let mut project = EditorProject::from(pool);
        project.mask_size = 500;
        project
    }

    #[test]
    fn move_resize_remove_and_undo_are_one_atomic_canvas_action() {
        let mut pool = ObjectPool::new();
        pool.add(object_with_id(ObjectType::DataMask, 1));
        pool.add(object_with_id(ObjectType::Button, 2));
        add_positioned_child(&mut pool, 1, 2, 0, 0);
        let mut project = project(pool);
        apply(
            &mut project,
            "Move and resize",
            &[
                CanvasEdit::Move {
                    target: PlacementTarget::Primitive {
                        parent_id: 1,
                        child_index: 0,
                    },
                    x: 4,
                    y: 9,
                },
                CanvasEdit::Resize {
                    object_id: 2,
                    width: Some(80),
                    height: None,
                },
            ],
        )
        .unwrap();
        assert_eq!(child_placements(project.get_pool(), 1).unwrap()[0].x, 4);
        assert!(
            matches!(project.get_pool().object_by_id(object_id(2).unwrap()), Some(Object::Button(button)) if button.width == 80)
        );
        assert_eq!(
            project
                .operation_history
                .borrow()
                .undo_description()
                .as_deref(),
            Some("Move and resize")
        );
        assert!(project.undo_operation());
        assert_eq!(child_placements(project.get_pool(), 1).unwrap()[0].x, 0);
        assert!(
            matches!(project.get_pool().object_by_id(object_id(2).unwrap()), Some(Object::Button(button)) if button.width == 0)
        );
    }

    #[test]
    fn duplicate_preserves_pointer_allowed_targets_and_remaps_copied_targets() {
        let mut pool = ObjectPool::new();
        pool.add(object_with_id(ObjectType::DataMask, 1));
        pool.add(object_with_id(ObjectType::Container, 2));
        pool.add(object_with_id(ObjectType::ObjectPointer, 3));
        pool.add(object_with_id(ObjectType::Button, 4));
        pool.add(object_with_id(ObjectType::Button, 5));
        add_positioned_child(&mut pool, 1, 2, 0, 0);
        add_positioned_child(&mut pool, 2, 3, 0, 0);
        add_positioned_child(&mut pool, 2, 4, 0, 0);
        let mut project = project(pool);
        project.queue_operation(Operation::SetObjectPointerAllowedReferences {
            object: ObjectTarget::id(3),
            references: vec![ObjectTarget::id(4), ObjectTarget::id(5)],
        });
        assert!(project.commit_queued("Allow pointer targets"));
        apply(
            &mut project,
            "Duplicate container",
            &[CanvasEdit::Duplicate {
                target: PlacementTarget::Primitive {
                    parent_id: 1,
                    child_index: 0,
                },
                x: 0,
                y: 0,
            }],
        )
        .unwrap();
        let root = target_id(&child_placements(project.get_pool(), 1).unwrap()[1].child).unwrap();
        let children = child_placements(project.get_pool(), root).unwrap();
        let pointer = target_id(&children[0].child).unwrap();
        let copied_target = target_id(&children[1].child).unwrap();
        let metadata = project.component_object_info();
        assert_eq!(
            metadata[&object_id(pointer).unwrap()].allowed_references,
            vec![object_id(copied_target).unwrap(), object_id(5).unwrap()]
        );
        assert_eq!(
            metadata[&object_id(3).unwrap()].allowed_references,
            vec![object_id(4).unwrap(), object_id(5).unwrap()]
        );
    }

    #[test]
    fn duplicate_copies_positioned_subtree_and_remaps_internal_references() {
        let mut pool = ObjectPool::new();
        pool.add(object_with_id(ObjectType::DataMask, 1));
        let mut container = object_with_id(ObjectType::Container, 2);
        if let Object::Container(container) = &mut container {
            container.width = 100;
            container.height = 100;
        }
        pool.add(container);
        pool.add(object_with_id(ObjectType::Button, 3));
        pool.add(object_with_id(ObjectType::Button, 4));
        add_positioned_child(&mut pool, 1, 2, 2, 3);
        add_positioned_child(&mut pool, 2, 3, 4, 5);
        let mut project = project(pool);
        project
            .edit_source("Seed object labels", |pool, _, _, info| {
                let mut original = ObjectInfo::new(pool.object_by_id(object_id(2)?).unwrap());
                original.name = Some("control".to_owned());
                original.description = Some("Reusable control".to_owned());
                original.tags = vec!["input".to_owned()];
                original.id_overridden = true;
                info.insert(object_id(2)?, original);
                let mut collision = ObjectInfo::new(pool.object_by_id(object_id(4)?).unwrap());
                collision.name = Some("control copy".to_owned());
                info.insert(object_id(4)?, collision);
                Ok(())
            })
            .unwrap();
        let original_metadata = project.component_object_info()[&object_id(2).unwrap()].clone();
        apply(
            &mut project,
            "Place original",
            &[CanvasEdit::Duplicate {
                target: PlacementTarget::Primitive {
                    parent_id: 1,
                    child_index: 0,
                },
                x: 10,
                y: 11,
            }],
        )
        .unwrap();
        let children = child_placements(project.get_pool(), 1).unwrap();
        assert_eq!(children.len(), 2);
        let copied_root = target_id(&children[1].child).unwrap();
        assert_ne!(copied_root, 2);
        assert_eq!((children[1].x, children[1].y), (10, 11));
        let copied = project
            .get_pool()
            .object_by_id(object_id(copied_root).unwrap())
            .unwrap();
        let copied_child = object_refs(copied).unwrap()[0].id.value();
        assert_ne!(copied_child, 3);
        assert!(project
            .get_pool()
            .object_by_id(object_id(copied_child).unwrap())
            .is_some());
        let copied_metadata =
            project.component_object_info()[&object_id(copied_root).unwrap()].clone();
        assert_ne!(
            copied_metadata.get_unique_id(),
            original_metadata.get_unique_id()
        );
        assert_eq!(copied_metadata.name.as_deref(), Some("control copy 2"));
        assert_eq!(
            copied_metadata.description.as_deref(),
            Some("Reusable control")
        );
        assert_eq!(copied_metadata.tags, vec!["input"]);
        assert!(!copied_metadata.id_overridden);
    }

    #[test]
    fn failed_batch_does_not_change_project_or_history() {
        let mut pool = ObjectPool::new();
        pool.add(object_with_id(ObjectType::DataMask, 1));
        let mut project = project(pool);
        let before = project.get_pool().clone();
        assert!(apply(
            &mut project,
            "Invalid",
            &[CanvasEdit::Move {
                target: PlacementTarget::Primitive {
                    parent_id: 1,
                    child_index: 4
                },
                x: 1,
                y: 2,
            }]
        )
        .is_err());
        assert_eq!(project.get_pool(), &before);
        assert!(!project.operation_history.borrow().can_undo());
    }

    #[test]
    fn unchanged_move_does_not_create_history() {
        let mut pool = ObjectPool::new();
        pool.add(object_with_id(ObjectType::DataMask, 1));
        pool.add(object_with_id(ObjectType::Button, 2));
        add_positioned_child(&mut pool, 1, 2, 0, 0);
        let mut project = project(pool);
        apply(
            &mut project,
            "No-op move",
            &[CanvasEdit::Move {
                target: PlacementTarget::Primitive {
                    parent_id: 1,
                    child_index: 0,
                },
                x: 0,
                y: 0,
            }],
        )
        .unwrap();
        assert!(!project.operation_history.borrow().can_undo());
    }

    #[test]
    fn multiple_primitive_removals_keep_original_child_indices() {
        let mut pool = ObjectPool::new();
        pool.add(object_with_id(ObjectType::DataMask, 1));
        for id in [2, 3, 4] {
            pool.add(object_with_id(ObjectType::Button, id));
            add_positioned_child(&mut pool, 1, id, id as i16, 0);
        }
        let mut project = project(pool);
        apply(
            &mut project,
            "Remove two placements",
            &[
                CanvasEdit::Remove {
                    target: PlacementTarget::Primitive {
                        parent_id: 1,
                        child_index: 1,
                    },
                },
                CanvasEdit::Remove {
                    target: PlacementTarget::Primitive {
                        parent_id: 1,
                        child_index: 2,
                    },
                },
            ],
        )
        .unwrap();
        let remaining = child_placements(project.get_pool(), 1).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(target_id(&remaining[0].child), Some(2));
    }

    #[test]
    fn duplicate_remove_target_removes_only_one_occurrence() {
        let mut pool = ObjectPool::new();
        pool.add(object_with_id(ObjectType::DataMask, 1));
        for id in [2, 3] {
            pool.add(object_with_id(ObjectType::Button, id));
            add_positioned_child(&mut pool, 1, id, 0, 0);
        }
        let mut project = project(pool);
        let remove = CanvasEdit::Remove {
            target: PlacementTarget::Primitive {
                parent_id: 1,
                child_index: 0,
            },
        };
        apply(&mut project, "Repeated remove", &[remove.clone(), remove]).unwrap();
        let remaining = child_placements(project.get_pool(), 1).unwrap();
        assert_eq!(remaining.len(), 1);
        assert_eq!(target_id(&remaining[0].child), Some(3));
    }

    #[test]
    fn mixed_primitive_and_component_move_is_one_undoable_change() {
        let mut pool = ObjectPool::new();
        pool.add(object_with_id(ObjectType::DataMask, 1));
        pool.add(object_with_id(ObjectType::Button, 2));
        add_positioned_child(&mut pool, 1, 2, 0, 0);
        let mut project = project(pool);
        let instance_id = add_component_instance(&mut project, 1, 20, 30);
        apply(
            &mut project,
            "Move both placements",
            &[
                CanvasEdit::Move {
                    target: PlacementTarget::Primitive {
                        parent_id: 1,
                        child_index: 0,
                    },
                    x: 3,
                    y: 7,
                },
                CanvasEdit::Move {
                    target: PlacementTarget::Component { instance_id },
                    x: 40,
                    y: 12,
                },
            ],
        )
        .unwrap();
        assert_eq!(
            position(
                &project,
                &PlacementTarget::Primitive {
                    parent_id: 1,
                    child_index: 0
                }
            ),
            Some((3, 7))
        );
        assert_eq!(
            position(&project, &PlacementTarget::Component { instance_id }),
            Some((40, 12))
        );
        assert_eq!(
            project
                .operation_history
                .borrow()
                .undo_description()
                .as_deref(),
            Some("Move both placements")
        );
        assert!(project.undo_operation());
        assert_eq!(
            position(
                &project,
                &PlacementTarget::Primitive {
                    parent_id: 1,
                    child_index: 0
                }
            ),
            Some((0, 0))
        );
        assert_eq!(
            position(&project, &PlacementTarget::Component { instance_id }),
            Some((20, 30))
        );
    }

    #[test]
    fn component_duplication_and_removals_preserve_display_order_and_targets() {
        let mut pool = ObjectPool::new();
        pool.add(object_with_id(ObjectType::DataMask, 1));
        for (id, x) in [(2, 1), (3, 2)] {
            pool.add(object_with_id(ObjectType::Button, id));
            add_positioned_child(&mut pool, 1, id, x, 0);
        }
        let mut project = project(pool);
        let original_instance = add_component_instance(&mut project, 1, 4, 5);
        project
            .edit_components("Place component between buttons", |_, source| {
                source
                    .instances
                    .iter_mut()
                    .find(|instance| instance.id == original_instance)
                    .unwrap()
                    .child_index = Some(1);
                Ok(())
            })
            .unwrap();
        let duplicated_targets = apply(
            &mut project,
            "Duplicate, move and remove",
            &[
                CanvasEdit::Duplicate {
                    target: PlacementTarget::Component {
                        instance_id: original_instance,
                    },
                    x: 8,
                    y: 9,
                },
                CanvasEdit::Move {
                    target: PlacementTarget::Primitive {
                        parent_id: 1,
                        child_index: 1,
                    },
                    x: 77,
                    y: 6,
                },
                CanvasEdit::Remove {
                    target: PlacementTarget::Primitive {
                        parent_id: 1,
                        child_index: 0,
                    },
                },
            ],
        )
        .unwrap();
        let duplicate_instance = match duplicated_targets[0] {
            PlacementTarget::Component { instance_id } => instance_id,
            _ => panic!("expected duplicated component selection"),
        };
        assert_ne!(duplicate_instance, original_instance);
        assert_eq!(
            duplicated_targets[1],
            PlacementTarget::Primitive {
                parent_id: 1,
                child_index: 0
            }
        );
        assert_eq!(position(&project, &duplicated_targets[1]), Some((77, 6)));
        let materialized = project.materialize().unwrap();
        let display_ids: Vec<_> = object_refs(
            materialized
                .pool
                .object_by_id(object_id(1).unwrap())
                .unwrap(),
        )
        .unwrap()
        .iter()
        .map(|reference| reference.id.value())
        .collect();
        let original_root = materialized.source_map.by_component_node[&(
            original_instance,
            project
                .component_source()
                .definitions
                .values()
                .next()
                .unwrap()
                .root_node,
        )];
        let duplicate_root = materialized.source_map.by_component_node[&(
            duplicate_instance,
            project
                .component_source()
                .definitions
                .values()
                .next()
                .unwrap()
                .root_node,
        )];
        assert_eq!(display_ids, vec![original_root, 3, duplicate_root]);

        apply(
            &mut project,
            "Remove original component",
            &[CanvasEdit::Remove {
                target: PlacementTarget::Component {
                    instance_id: original_instance,
                },
            }],
        )
        .unwrap();
        assert_eq!(project.component_source().instances.len(), 1);
        assert_eq!(
            project.component_source().instances[0].id,
            duplicate_instance
        );
        assert_eq!(project.component_source().instances[0].child_index, Some(1));
        let after_removal = project.materialize().unwrap();
        let ids: Vec<_> = object_refs(
            after_removal
                .pool
                .object_by_id(object_id(1).unwrap())
                .unwrap(),
        )
        .unwrap()
        .iter()
        .map(|reference| reference.id.value())
        .collect();
        let surviving_root = after_removal.source_map.by_component_node[&(
            duplicate_instance,
            project
                .component_source()
                .definitions
                .values()
                .next()
                .unwrap()
                .root_node,
        )];
        assert_eq!(ids, vec![3, surviving_root]);
    }

    fn add_component_instance(
        project: &mut EditorProject,
        parent_id: u16,
        x: i32,
        y: i32,
    ) -> ComponentInstanceId {
        use crate::components::ComponentNode;
        let component_id = ComponentId::default();
        let root_node = ComponentNodeId::default();
        ComponentService::edit_component_definition(
            project,
            ComponentDefinition {
                schema_version: 1,
                id: component_id,
                name: "Canvas test component".to_owned(),
                description: String::new(),
                category_path: vec![],
                tags: vec![],
                revision: 0,
                root_node,
                nodes: vec![ComponentNode {
                    id: root_node,
                    name: "Root".to_owned(),
                    object_type: ObjectType::OutputRectangle,
                    properties: BTreeMap::new(),
                    children: vec![],
                    references: BTreeMap::new(),
                    macro_references: vec![],
                    object_list: None,
                }],
                parameters: vec![],
            },
        )
        .unwrap();
        ComponentService::instantiate_component(
            project,
            component_id,
            parent_id,
            x,
            y,
            BTreeMap::new(),
        )
        .unwrap()
    }

    fn add_positioned_child(pool: &mut ObjectPool, parent: u16, child: u16, x: i16, y: i16) {
        use ag_iso_stack::object_pool::object_attributes::Point;
        use ag_iso_stack::object_pool::ObjectRef;
        let object = pool
            .object_mut_by_id(object_id(parent).unwrap())
            .expect("test parent exists");
        let reference = ObjectRef {
            id: object_id(child).unwrap(),
            offset: Point { x, y },
        };
        match object {
            Object::DataMask(mask) => mask.object_refs.push(reference),
            Object::Container(container) => container.object_refs.push(reference),
            _ => panic!("unsupported test parent"),
        }
    }
}
