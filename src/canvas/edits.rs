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

#[derive(Clone)]
enum DisplayedSibling {
    Primitive(ChildPlacement),
    Component(crate::components::ComponentInstance),
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

/// Return the canvas origin for children of a canonical hierarchy occurrence.
/// Paths use primitive child-list indices, with `usize::MAX` entering an
/// ObjectPointer target, as displayed by the editor tree. Button face insets are
/// included. The returned origin is mask-local and independent of viewport pan.
pub fn hierarchy_child_origin(
    project: &EditorProject,
    root: u16,
    path: &[usize],
) -> Result<egui::Pos2, String> {
    use ag_iso_stack::object_pool::object::Object;
    let mut id = root;
    let mut origin = egui::Pos2::ZERO;
    for &index in path {
        let object = project
            .get_pool()
            .object_by_id(object_id(id).map_err(|e| e.to_string())?)
            .ok_or("Missing hierarchy object")?;
        if index == usize::MAX {
            let Object::ObjectPointer(pointer) = object else {
                return Err("Invalid pointer hierarchy path".into());
            };
            id = pointer.value.0.ok_or("Empty pointer target")?.value();
        } else {
            if matches!(object, Object::Button(_)) {
                origin += egui::vec2(
                    crate::object_rendering::BUTTON_BORDER_INSET,
                    crate::object_rendering::BUTTON_BORDER_INSET,
                );
            }
            let child = object_refs(object)
                .and_then(|children| children.get(index))
                .ok_or("Invalid hierarchy path")?;
            origin += egui::vec2(f32::from(child.offset.x), f32::from(child.offset.y));
            id = child.id.value();
        }
    }
    let parent = project
        .get_pool()
        .object_by_id(object_id(id).map_err(|e| e.to_string())?)
        .ok_or("Missing destination object")?;
    if matches!(parent, Object::Button(_)) {
        origin += egui::vec2(
            crate::object_rendering::BUTTON_BORDER_INSET,
            crate::object_rendering::BUTTON_BORDER_INSET,
        );
    }
    Ok(origin)
}

/// Adjust a synthetic hierarchy parent path for the canonical child-index
/// shift caused by removing `target`. Paths start with `[0]` for the mask or
/// `[soft_key_slot + 1]` for soft keys; `usize::MAX` follows a pointer alias.
/// Component removal does not affect primitive indexes.
pub fn hierarchy_path_after_removal(
    project: &EditorProject,
    root: u16,
    path: &[usize],
    target: &PlacementTarget,
) -> Result<Vec<usize>, String> {
    use ag_iso_stack::object_pool::object::Object;

    let mut adjusted = path.to_vec();
    let PlacementTarget::Primitive {
        parent_id: source_parent,
        child_index: source_index,
    } = target
    else {
        return Ok(adjusted);
    };
    let Some(first) = path.first() else {
        return Err("Empty hierarchy path".into());
    };
    let (mut parent_id, start) = if *first == 0 {
        (root, 1)
    } else {
        let root_object = project
            .get_pool()
            .object_by_id(object_id(root).map_err(|e| e.to_string())?)
            .ok_or("Missing hierarchy root")?;
        let softkey_mask_id = match root_object {
            Object::DataMask(mask) => mask.soft_key_mask.0.ok_or("Missing soft-key mask")?.value(),
            Object::AlarmMask(mask) => mask.soft_key_mask.0.ok_or("Missing soft-key mask")?.value(),
            _ => return Err("Invalid soft-key hierarchy root".into()),
        };
        let softkey_mask = project
            .get_pool()
            .object_by_id(object_id(softkey_mask_id).map_err(|e| e.to_string())?)
            .ok_or("Missing soft-key mask")?;
        let Object::SoftKeyMask(softkey_mask) = softkey_mask else {
            return Err("Invalid soft-key mask".into());
        };
        let slot = *first - 1;
        let object_id = softkey_mask
            .objects
            .get(slot)
            .ok_or("Invalid soft-key slot")?
            .value();
        (object_id, 1)
    };

    for (path_index, segment) in path.iter().copied().enumerate().skip(start) {
        if segment == usize::MAX {
            let pointer = project
                .get_pool()
                .object_by_id(object_id(parent_id).map_err(|e| e.to_string())?)
                .ok_or("Missing hierarchy object")?;
            let Object::ObjectPointer(pointer) = pointer else {
                return Err("Invalid pointer hierarchy path".into());
            };
            parent_id = pointer.value.0.ok_or("Empty pointer target")?.value();
            continue;
        }
        if parent_id == *source_parent && segment > *source_index {
            adjusted[path_index] = segment - 1;
        }
        let parent = project
            .get_pool()
            .object_by_id(object_id(parent_id).map_err(|e| e.to_string())?)
            .ok_or("Missing hierarchy object")?;
        parent_id = object_refs(parent)
            .and_then(|children| children.get(segment))
            .ok_or("Invalid hierarchy path")?
            .id
            .value();
    }
    Ok(adjusted)
}

/// Reparent a tree occurrence while preserving its mask-local canvas position.
/// Source and destination paths identify primitive parent occurrences, avoiding
/// ambiguity when the same parent is placed more than once. Between masks, the
/// position is retained relative to each mask's origin, rather than dock layout.
pub fn reparent_in_hierarchy(
    project: &mut EditorProject,
    target: &PlacementTarget,
    source_root: u16,
    source_parent_path: &[usize],
    dest_root: u16,
    dest_parent_path: &[usize],
    new_parent: u16,
) -> Result<PlacementTarget, String> {
    reparent_in_hierarchy_at(
        project,
        target,
        source_root,
        source_parent_path,
        dest_root,
        dest_parent_path,
        new_parent,
        None,
    )
}

/// Reparent an occurrence at a combined primitive/component sibling slot.
/// `insertion_index` is a slot in the destination's displayed order before the
/// source is removed; `None` appends. Canvas position is preserved.
pub fn reparent_in_hierarchy_at(
    project: &mut EditorProject,
    target: &PlacementTarget,
    source_root: u16,
    source_parent_path: &[usize],
    dest_root: u16,
    dest_parent_path: &[usize],
    new_parent: u16,
    insertion_index: Option<usize>,
) -> Result<PlacementTarget, String> {
    let source_origin = hierarchy_child_origin(project, source_root, source_parent_path)?;
    let destination_origin = hierarchy_child_origin(project, dest_root, dest_parent_path)?;
    let (x, y) = position(project, target).ok_or("Missing source occurrence")?;
    let local = source_origin + egui::vec2(f32::from(x), f32::from(y)) - destination_origin;
    reparent_at_index(
        project,
        target,
        new_parent,
        local.x.round() as i32,
        local.y.round() as i32,
        insertion_index,
    )
}

/// Move one exact occurrence into a primitive positioned-child parent.
/// Local coordinates are retained where possible and clamped to fit the new
/// parent's viewport. Oversized children, invalid relationships and cycles fail
/// atomically; component instances retain their identities and overrides.
pub fn reparent(
    project: &mut EditorProject,
    target: &PlacementTarget,
    new_parent: u16,
) -> Result<PlacementTarget, String> {
    let (x, y) = position(project, target).ok_or("Missing source occurrence")?;
    reparent_at(project, target, new_parent, i32::from(x), i32::from(y))
}

/// Reparent an occurrence at the requested destination-local coordinates.
/// Callers preserving its canvas position subtract the destination child origin
/// from the source canvas position before calling. Coordinates are clamped to
/// fit the destination; failures leave source, destination and history intact.
pub fn reparent_at(
    project: &mut EditorProject,
    target: &PlacementTarget,
    new_parent: u16,
    x: i32,
    y: i32,
) -> Result<PlacementTarget, String> {
    reparent_at_index(project, target, new_parent, x, y, None)
}

/// Move an occurrence and place it at a combined primitive/component sibling
/// slot. `insertion_index` is measured before source removal; `None` appends.
/// Same-parent moves retain their coordinates, while parent changes preserve
/// the existing clamping behavior.
pub fn reparent_at_index(
    project: &mut EditorProject,
    target: &PlacementTarget,
    new_parent: u16,
    x: i32,
    y: i32,
    insertion_index: Option<usize>,
) -> Result<PlacementTarget, String> {
    use ag_iso_stack::object_pool::object::Object;
    let materialized = project.materialize().map_err(|error| error.to_string())?;
    let (child_id, old_parent) = match target {
        PlacementTarget::Primitive {
            parent_id,
            child_index,
        } => {
            let placement =
                child_placements(project.get_pool(), *parent_id).map_err(|e| e.to_string())?;
            let child = placement
                .get(*child_index)
                .and_then(|p| target_id(&p.child))
                .ok_or("Missing source occurrence")?;
            (child, *parent_id)
        }
        PlacementTarget::Component { instance_id } => {
            let instance = project
                .component_source()
                .instances
                .iter()
                .find(|instance| instance.id == *instance_id)
                .ok_or("Missing component occurrence")?;
            let definition = &project.component_source().definitions[&instance.component_id];
            let child = *materialized
                .source_map
                .by_component_node
                .get(&(*instance_id, definition.root_node))
                .ok_or("Missing component root")?;
            let EditorParentRef::Primitive(parent) = instance.parent;
            (child, parent)
        }
    };
    let same_parent = old_parent == new_parent;
    let old_display_index = if same_parent {
        Some(match target {
            PlacementTarget::Primitive { parent_id, child_index } => {
                primitive_display_index(project.get_pool(), project.component_source(), *parent_id, *child_index)
                    .map_err(|e| e.to_string())?
            }
            PlacementTarget::Component { instance_id } => displayed_siblings(
                project.get_pool(), project.component_source(), new_parent,
            ).map_err(|error| error.to_string())?
            .iter()
            .position(|sibling| matches!(sibling, DisplayedSibling::Component(instance) if instance.id == *instance_id))
            .ok_or("Missing component occurrence")?,
        })
    } else {
        None
    };
    let parent = project
        .get_pool()
        .object_by_id(object_id(new_parent).map_err(|e| e.to_string())?)
        .ok_or("Missing destination parent")?;
    if object_refs(parent).is_none() {
        return Err(format!(
            "Destination {} (ID {new_parent}) does not support positioned children",
            project.get_object_info(parent).get_name(parent)
        ));
    }
    let (mut width, mut height) = match parent {
        Object::DataMask(_) | Object::AlarmMask(_) => (project.mask_size, project.mask_size),
        Object::Key(_) => project.soft_key_size(),
        _ => project.get_pool().content_size(parent),
    };
    if matches!(parent, Object::Button(_)) {
        let inset = (crate::object_rendering::BUTTON_BORDER_INSET * 2.0) as u16;
        width = width.saturating_sub(inset);
        height = height.saturating_sub(inset);
    }
    let child = materialized
        .pool
        .object_by_id(object_id(child_id).map_err(|e| e.to_string())?)
        .ok_or("Missing source object")?;
    let (child_width, child_height) = materialized.pool.content_size(child);
    if !same_parent && (child_width > width || child_height > height) {
        return Err(format!(
            "Object {} (ID {child_id}) is too large for destination {} (ID {new_parent})",
            project.get_object_info(child).get_name(child),
            project.get_object_info(parent).get_name(parent)
        ));
    }
    let (x, y) = if same_parent {
        position(project, target).ok_or("Missing source occurrence")?
    } else {
        (
            x.clamp(0, i32::from(width - child_width))
                .min(i32::from(i16::MAX)) as i16,
            y.clamp(0, i32::from(height - child_height))
                .min(i32::from(i16::MAX)) as i16,
        )
    };
    if same_parent {
        let sibling_count =
            displayed_siblings(project.get_pool(), project.component_source(), new_parent)
                .map_err(|error| error.to_string())?
                .len();
        let requested_slot = insertion_index.unwrap_or(sibling_count).min(sibling_count);
        let effective_slot = if old_display_index.is_some_and(|source| source < requested_slot) {
            requested_slot - 1
        } else {
            requested_slot
        }
        .min(sibling_count.saturating_sub(1));
        if old_display_index == Some(effective_slot) {
            return Ok(target.clone());
        }
    }
    let version = project.vt_version;
    let mut result = target.clone();
    project
        .edit_source(
            "Move placement to parent",
            |pool, source, _graphics, _info| {
                let destination_count = displayed_siblings(pool, source, new_parent)?.len();
                let moved = match target {
                    PlacementTarget::Primitive {
                        parent_id,
                        child_index,
                    } => {
                        let placement = child_placements(pool, *parent_id)?
                            .get(*child_index)
                            .cloned()
                            .ok_or_else(invalid_placement)?;
                        DisplayedSibling::Primitive(ChildPlacement { x, y, ..placement })
                    }
                    PlacementTarget::Component { instance_id } => {
                        let mut instance = source
                            .instances
                            .iter()
                            .find(|instance| instance.id == *instance_id)
                            .cloned()
                            .ok_or_else(invalid_placement)?;
                        instance.x = i32::from(x);
                        instance.y = i32::from(y);
                        DisplayedSibling::Component(instance)
                    }
                };
                remove_target(pool, source, target, version)?;
                let mut siblings = displayed_siblings(pool, source, new_parent)?;
                let slot = insertion_index.unwrap_or(destination_count);
                let slot = if same_parent && old_display_index.is_some_and(|source| source < slot) {
                    slot - 1
                } else {
                    slot
                };
                let slot = slot.min(siblings.len());
                siblings.insert(slot, moved);
                set_displayed_siblings(pool, source, new_parent, siblings, version)?;
                result = match target {
                    PlacementTarget::Primitive { .. } => PlacementTarget::Primitive {
                        parent_id: new_parent,
                        child_index: displayed_siblings(pool, source, new_parent)?[..slot]
                            .iter()
                            .filter(|sibling| matches!(sibling, DisplayedSibling::Primitive(_)))
                            .count(),
                    },
                    PlacementTarget::Component { .. } => target.clone(),
                };
                Ok(())
            },
        )
        .map_err(|error| error.to_string())?;
    Ok(result)
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
            let new_root = mapping.get(&root_id).copied().unwrap_or(root_id);
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
                    metadata.notes = original.notes.clone();
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
    // Image data is a shared resource: duplicating placements and containers
    // retains PictureGraphic identities rather than copying their pixel payload.
    if matches!(
        object,
        ag_iso_stack::object_pool::object::Object::PictureGraphic(_)
    ) {
        return Ok(());
    }
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

fn displayed_siblings(
    pool: &ObjectPool,
    source: &ComponentSource,
    parent_id: u16,
) -> ComponentResult<Vec<DisplayedSibling>> {
    let mut siblings: Vec<_> = child_placements(pool, parent_id)?
        .into_iter()
        .map(DisplayedSibling::Primitive)
        .collect();
    let mut instances: Vec<_> = source
        .instances
        .iter()
        .filter(|instance| instance.parent == EditorParentRef::Primitive(parent_id))
        .collect();
    instances.sort_by_key(|instance| (instance.child_index.unwrap_or(usize::MAX), instance.id));
    for instance in instances {
        let index = instance
            .child_index
            .unwrap_or(siblings.len())
            .min(siblings.len());
        siblings.insert(index, DisplayedSibling::Component(instance.clone()));
    }
    Ok(siblings)
}

fn set_displayed_siblings(
    pool: &mut ObjectPool,
    source: &mut ComponentSource,
    parent_id: u16,
    siblings: Vec<DisplayedSibling>,
    version: VtVersion,
) -> ComponentResult<()> {
    let mut placements = Vec::new();
    for (display_index, sibling) in siblings.into_iter().enumerate() {
        match sibling {
            DisplayedSibling::Primitive(placement) => placements.push(placement),
            DisplayedSibling::Component(moved) => {
                let instance = if let Some(instance) = source
                    .instances
                    .iter_mut()
                    .find(|instance| instance.id == moved.id)
                {
                    instance
                } else {
                    source.instances.push(moved.clone());
                    source.instances.last_mut().ok_or_else(invalid_placement)?
                };
                instance.parent = EditorParentRef::Primitive(parent_id);
                instance.child_index = Some(display_index);
                instance.x = moved.x;
                instance.y = moved.y;
            }
        }
    }
    set_child_placements(pool, parent_id, placements, version)
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
