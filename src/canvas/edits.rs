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
use ag_iso_stack::object_pool::{object_attributes::LineDirection, ObjectId, ObjectPool};
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

/// A possible parent type for the current grouping selection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GroupingCandidate {
    /// Object type to create when enabled.
    pub object_type: ag_iso_stack::object_pool::ObjectType,
    /// Whether grouping with this type currently validates.
    pub enabled: bool,
    /// Why the type cannot currently be used.
    pub reason: Option<String>,
}

#[derive(Clone)]
enum DisplayedSibling {
    Primitive(ChildPlacement),
    Component(crate::components::ComponentInstance),
}

/// An edit to a placement or shared shape geometry.
/// Positions are signed parent-relative pool pixels. Dimensions and polygon
/// points belong to the shared object definition.
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
    SetPolygonPoint {
        object_id: u16,
        target: PlacementTarget,
        index: usize,
        x: u16,
        y: u16,
    },
    EditLineEndpoint {
        object_id: u16,
        target: PlacementTarget,
        x: i16,
        y: i16,
        width: u16,
        height: u16,
        direction: LineDirection,
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
/// Movement, duplication, and shape edits supply selection targets; resize alone
/// supplies none.
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
                    CanvasEdit::SetPolygonPoint { object_id: id, target, index, x, y } => {
                        let Some(ag_iso_stack::object_pool::object::Object::OutputPolygon(polygon)) =
                            pool.object_mut_by_id(object_id(*id)?)
                        else {
                            return Err(ComponentError::new(
                                "CANVAS_EDIT_INVALID",
                                "Selected object is not an output polygon",
                            ));
                        };
                        let Some(point) = polygon.points.get_mut(*index) else {
                            return Err(ComponentError::new(
                                "CANVAS_EDIT_INVALID",
                                "Polygon point no longer exists",
                            ));
                        };
                        point.x = *x;
                        point.y = *y;
                        if !touched.contains(target) {
                            touched.push(target.clone());
                        }
                    }
                    CanvasEdit::EditLineEndpoint {
                        object_id: line_id,
                        target,
                        x,
                        y,
                        width,
                        height,
                        direction,
                    } => {
                        move_target(pool, source, target, *x, *y, vt_version)?;
                        let Some(ag_iso_stack::object_pool::object::Object::OutputLine(line)) =
                            pool.object_mut_by_id(object_id(*line_id)?)
                        else {
                            return Err(ComponentError::new(
                                "CANVAS_EDIT_INVALID",
                                "Selected object is not an output line",
                            ));
                        };
                        line.width = *width;
                        line.height = *height;
                        line.line_direction = *direction;
                        if !touched.contains(target) {
                            touched.push(target.clone());
                        }
                    }
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
            origin += egui::vec2(
                crate::button_geometry::object_face_inset(object, project.vt_version),
                crate::button_geometry::object_face_inset(object, project.vt_version),
            );
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
    let inset = crate::button_geometry::object_face_inset(parent, project.vt_version);
    origin += egui::vec2(inset, inset);
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
    hierarchy_path_after_removals(project, root, path, std::slice::from_ref(target))
}

/// Adjust a synthetic hierarchy path after removing several occurrences.
/// Primitive indexes are interpreted against the original hierarchy at each
/// level; all removed preceding primitive siblings are counted together.
/// Component removals do not shift primitive indexes. A path into an occurrence
/// being removed is rejected.
pub fn hierarchy_path_after_removals(
    project: &EditorProject,
    root: u16,
    path: &[usize],
    targets: &[PlacementTarget],
) -> Result<Vec<usize>, String> {
    use ag_iso_stack::object_pool::object::Object;

    if !targets
        .iter()
        .any(|target| matches!(target, PlacementTarget::Primitive { .. }))
    {
        return Ok(path.to_vec());
    }
    let mut adjusted = path.to_vec();
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
        let mut removed_before = 0;
        for target in targets {
            if let PlacementTarget::Primitive {
                parent_id: source_parent,
                child_index: source_index,
            } = target
            {
                if parent_id == *source_parent {
                    if segment == *source_index {
                        return Err("Destination is inside an occurrence being removed".into());
                    }
                    if *source_index < segment {
                        removed_before += 1;
                    }
                }
            }
        }
        adjusted[path_index] = segment.saturating_sub(removed_before);
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
        let inset =
            (crate::button_geometry::object_face_inset(parent, project.vt_version) * 2.0) as u16;
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

/// Return version-compatible parent choices with a reason for each rejected
/// choice. Candidate checks run the real grouping edit on a project copy, so
/// geometry, relationships, ID ranges and normal project validation agree with
/// execution.
pub fn grouping_candidates(
    project: &EditorProject,
    targets: &[PlacementTarget],
) -> Vec<GroupingCandidate> {
    use ag_iso_stack::object_pool::ObjectType;
    if targets.is_empty() {
        return Vec::new();
    }
    let Ok(parent_id) = targets[0].parent_id(project) else {
        return Vec::new();
    };
    if targets
        .iter()
        .any(|target| target.parent_id(project).ok() != Some(parent_id))
    {
        return Vec::new();
    }
    let Ok(parent_object_id) = object_id(parent_id) else {
        return Vec::new();
    };
    let Some(existing_parent) = project.get_pool().object_by_id(parent_object_id) else {
        return Vec::new();
    };
    let parent_allowed = crate::allowed_object_relationships::get_allowed_child_refs(
        existing_parent.object_type(),
        project.vt_version,
    );
    let Ok(siblings) =
        displayed_siblings(project.get_pool(), project.component_source(), parent_id)
    else {
        return Vec::new();
    };
    let Ok(materialized) = project.materialize() else {
        return Vec::new();
    };
    let mut selected_types = Vec::new();
    for (index, sibling) in siblings.iter().enumerate() {
        if targets
            .iter()
            .any(|target| sibling_matches_target(&siblings, index, target))
        {
            let Ok(child_id) =
                displayed_child_id(sibling, &materialized, project.component_source())
            else {
                return Vec::new();
            };
            let Ok(child_id) = object_id(child_id) else {
                return Vec::new();
            };
            let Some(child) = materialized.pool.object_by_id(child_id) else {
                return Vec::new();
            };
            selected_types.push(child.object_type());
        }
    }
    ObjectType::values()
        .into_iter()
        .filter(|object_type| {
            crate::vt_compatibility::supported_object_type(*object_type, project.vt_version)
                && crate::operations::object_refs(&crate::object_defaults::default_object(
                    *object_type,
                ))
                .is_some()
                && parent_allowed.contains(object_type)
                && selected_types.iter().all(|child_type| {
                    crate::allowed_object_relationships::get_allowed_child_refs(
                        *object_type,
                        project.vt_version,
                    )
                    .contains(child_type)
                })
        })
        .map(|object_type| {
            let mut preview = project.clone();
            let result = group_placements(&mut preview, targets, object_type, "Grouping preview")
                .map(|_| ());
            GroupingCandidate {
                object_type,
                enabled: result.is_ok(),
                reason: result.err(),
            }
        })
        .collect()
}

/// Group one or more sibling occurrences under a newly-created parent. The
/// selected children retain their positions in the parent’s canvas coordinate
/// system, and their displayed order is retained. The new parent is inserted
/// at the first selected sibling’s position. The entire operation is one
/// validated history change.
pub fn group_placements(
    project: &mut EditorProject,
    targets: &[PlacementTarget],
    parent_type: ag_iso_stack::object_pool::ObjectType,
    name: &str,
) -> Result<PlacementTarget, String> {
    use ag_iso_stack::object_pool::{object::Object, ObjectType};
    if targets.is_empty() {
        return Err("Select at least one sibling occurrence".into());
    }
    if !crate::vt_compatibility::supported_object_type(parent_type, project.vt_version) {
        return Err(format!("{parent_type:?} is unavailable in this VT version"));
    }
    let parent_template =
        crate::object_defaults::default_object_for_version(parent_type, project.vt_version)
            .ok_or_else(|| format!("{parent_type:?} is unavailable in this VT version"))?;
    if crate::operations::object_refs(&parent_template).is_none() {
        return Err(format!(
            "{parent_type:?} does not support positioned children"
        ));
    }
    let parent_id = targets[0].parent_id(project)?;
    if targets
        .iter()
        .any(|target| target.parent_id(project).ok() != Some(parent_id))
    {
        return Err("Selected occurrences must be siblings under the same parent".into());
    }
    let siblings = displayed_siblings(project.get_pool(), project.component_source(), parent_id)
        .map_err(|error| error.to_string())?;
    let mut selected = Vec::new();
    for (display_index, sibling) in siblings.iter().enumerate() {
        if targets
            .iter()
            .any(|target| sibling_matches_target(&siblings, display_index, target))
        {
            selected.push((display_index, sibling.clone()));
        }
    }
    if selected.len() != targets.iter().collect::<HashSet<_>>().len() {
        return Err("A selected occurrence no longer exists".into());
    }
    if selected.len() != targets.len() {
        return Err("The same occurrence was selected more than once".into());
    }
    if parent_type == ObjectType::Animation && selected.len() > usize::from(u8::MAX) + 1 {
        return Err("An Animation can contain at most 256 selected frames".into());
    }
    let materialized = project.materialize().map_err(|error| error.to_string())?;
    let mut child_bounds = Vec::new();
    for (_, sibling) in &selected {
        let child_id = displayed_child_id(sibling, &materialized, project.component_source())?;
        let child = materialized
            .pool
            .object_by_id(object_id(child_id).map_err(|e| e.to_string())?)
            .ok_or("Missing selected object")?;
        let (width, height) = occurrence_size(&materialized.pool, child, project.soft_key_size());
        let width = u16::try_from(width).map_err(|_| "Selected object width is out of range")?;
        let height = u16::try_from(height).map_err(|_| "Selected object height is out of range")?;
        let (x, y) = displayed_position(sibling);
        child_bounds.push((x, y, width, height));
    }
    let min_x = child_bounds.iter().map(|b| i32::from(b.0)).min().unwrap();
    let min_y = child_bounds.iter().map(|b| i32::from(b.1)).min().unwrap();
    let max_x = child_bounds
        .iter()
        .map(|b| i32::from(b.0) + i32::from(b.2))
        .max()
        .unwrap();
    let max_y = child_bounds
        .iter()
        .map(|b| i32::from(b.1) + i32::from(b.3))
        .max()
        .unwrap();
    let inset =
        crate::button_geometry::object_face_inset(&parent_template, project.vt_version) as i32;
    let required_width = u16::try_from(max_x - min_x + inset * 2)
        .map_err(|_| "Grouped parent width is out of range")?;
    let required_height = u16::try_from(max_y - min_y + inset * 2)
        .map_err(|_| "Grouped parent height is out of range")?;
    let (width, height) = match parent_type {
        ObjectType::DataMask | ObjectType::AlarmMask => (project.mask_size, project.mask_size),
        ObjectType::Key
        | ObjectType::WorkingSet
        | ObjectType::AuxiliaryFunctionType1
        | ObjectType::AuxiliaryInputType1
        | ObjectType::AuxiliaryFunctionType2
        | ObjectType::AuxiliaryInputType2 => project.soft_key_size(),
        _ => (required_width, required_height),
    };
    let content_width = width.saturating_sub((inset * 2) as u16);
    let content_height = height.saturating_sub((inset * 2) as u16);
    if max_x - min_x > i32::from(content_width) || max_y - min_y > i32::from(content_height) {
        return Err(format!(
            "Selected objects do not fit inside {parent_type:?}"
        ));
    }
    let parent_x =
        i16::try_from(min_x - inset).map_err(|_| "Grouped parent x position is out of range")?;
    let parent_y =
        i16::try_from(min_y - inset).map_err(|_| "Grouped parent y position is out of range")?;
    let positions: Vec<_> = child_bounds
        .iter()
        .map(|(x, y, _, _)| {
            let x = i16::try_from(i32::from(*x) - min_x)
                .map_err(|_| "Grouped child x position is out of range")?;
            let y = i16::try_from(i32::from(*y) - min_y)
                .map_err(|_| "Grouped child y position is out of range")?;
            Ok((x, y))
        })
        .collect::<Result<_, String>>()?;

    let version = project.vt_version;
    let parent_object_id = project
        .allocate_object_id(parent_type)
        .map_err(|error| error.to_string())?;
    let new_id = parent_object_id.value();
    let selected_targets = targets.to_vec();
    let mut result = None;
    project
        .edit_source("Group placements", |pool, source, _graphics, info| {
            let current = displayed_siblings(pool, source, parent_id)?;
            if selected_targets.iter().any(|target| {
                !current
                    .iter()
                    .enumerate()
                    .any(|(i, _)| sibling_matches_target(&current, i, target))
            }) {
                return Err(ComponentError::new(
                    "CANVAS_EDIT_INVALID",
                    "A selected occurrence no longer exists",
                ));
            }
            let mut parent = parent_template.clone();
            parent.mut_id().set_value(new_id).map_err(|e| {
                ComponentError::new("CANVAS_EDIT_INVALID", format!("Invalid object ID: {e:?}"))
            })?;
            match &mut parent {
                Object::Container(container) => {
                    container.width = width;
                    container.height = height;
                }
                Object::Button(button) => {
                    button.width = width;
                    button.height = height;
                }
                Object::Animation(animation) => {
                    animation.width = width;
                    animation.height = height;
                    animation.first_child_index = 0;
                    animation.last_child_index = (selected.len() - 1) as u8;
                    animation.default_child_index = 0;
                }
                _ => (),
            }
            pool.add(parent.clone());
            let parent_oid = parent_object_id;
            let parent_object = pool
                .object_by_id(parent_oid)
                .ok_or_else(invalid_placement)?;
            let mut metadata = ObjectInfo::new(parent_object);
            metadata.set_name(name.to_owned());
            info.insert(parent_oid, metadata);

            let mut child_siblings = Vec::new();
            for ((_, sibling), (x, y)) in selected.iter().zip(positions.iter().copied()) {
                match sibling {
                    DisplayedSibling::Primitive(placement) => {
                        child_siblings.push(DisplayedSibling::Primitive(ChildPlacement {
                            x,
                            y,
                            ..placement.clone()
                        }))
                    }
                    DisplayedSibling::Component(instance) => {
                        let mut moved = instance.clone();
                        moved.x = i32::from(x);
                        moved.y = i32::from(y);
                        moved.parent = EditorParentRef::Primitive(new_id);
                        child_siblings.push(DisplayedSibling::Component(moved));
                    }
                }
            }
            let new_children: Vec<_> = child_siblings
                .iter()
                .filter_map(|s| match s {
                    DisplayedSibling::Primitive(p) => Some(p.clone()),
                    _ => None,
                })
                .collect();
            for sibling in &child_siblings {
                let child_id = match sibling {
                    DisplayedSibling::Primitive(p) => {
                        target_id(&p.child).ok_or_else(invalid_placement)?
                    }
                    DisplayedSibling::Component(instance) => {
                        let definition = source
                            .definitions
                            .get(&instance.component_id)
                            .ok_or_else(invalid_placement)?;
                        *materialized
                            .source_map
                            .by_component_node
                            .get(&(instance.id, definition.root_node))
                            .ok_or_else(invalid_placement)?
                    }
                };
                let child_type = materialized
                    .pool
                    .object_by_id(object_id(child_id)?)
                    .ok_or_else(invalid_placement)?
                    .object_type();
                if !crate::allowed_object_relationships::get_allowed_child_refs(
                    parent_type,
                    version,
                )
                .contains(&child_type)
                {
                    return Err(ComponentError::new(
                        "CANVAS_EDIT_INVALID",
                        format!("{parent_type:?} cannot contain {child_type:?} for VT {version:?}"),
                    ));
                }
            }
            let existing_parent = pool
                .object_by_id(object_id(parent_id)?)
                .ok_or_else(invalid_placement)?;
            if !crate::allowed_object_relationships::get_allowed_child_refs(
                existing_parent.object_type(),
                version,
            )
            .contains(&parent_type)
            {
                return Err(ComponentError::new(
                    "CANVAS_EDIT_INVALID",
                    format!(
                        "{:?} cannot contain {parent_type:?} for VT {version:?}",
                        existing_parent.object_type()
                    ),
                ));
            }
            set_child_placements(pool, new_id, new_children, version)?;
            let mut remaining = current
                .iter()
                .enumerate()
                .filter(|(i, _)| {
                    !selected_targets
                        .iter()
                        .any(|t| sibling_matches_target(&current, *i, t))
                })
                .map(|(_, s)| s.clone())
                .collect::<Vec<_>>();
            let first_slot = selected
                .first()
                .map(|item| item.0)
                .unwrap_or(remaining.len())
                .min(remaining.len());
            remaining.insert(
                first_slot,
                DisplayedSibling::Primitive(ChildPlacement {
                    child: ObjectTarget::id(new_id),
                    x: parent_x,
                    y: parent_y,
                }),
            );
            set_displayed_siblings(pool, source, parent_id, remaining, version)?;
            set_displayed_siblings(pool, source, new_id, child_siblings, version)?;
            result = Some(PlacementTarget::Primitive {
                parent_id,
                child_index: child_placements(pool, parent_id)?
                    .iter()
                    .position(|p| target_id(&p.child) == Some(new_id))
                    .ok_or_else(invalid_placement)?,
            });
            Ok(())
        })
        .map_err(|error| error.to_string())?;
    result.ok_or_else(|| "Grouping did not create a parent occurrence".into())
}

/// Move selected sibling occurrences as one block, preserving their displayed
/// order and coordinates. Cross-parent moves apply `coordinate_delta` and fail
/// if any child would fall outside the destination’s content area.
pub fn move_placements(
    project: &mut EditorProject,
    targets: &[PlacementTarget],
    destination_parent: u16,
    insertion_index: Option<usize>,
    coordinate_delta: (i32, i32),
) -> Result<Vec<PlacementTarget>, String> {
    use ag_iso_stack::object_pool::object::Object;
    if targets.is_empty() {
        return Ok(Vec::new());
    }
    let source_parent = targets[0].parent_id(project)?;
    if targets
        .iter()
        .any(|target| target.parent_id(project).ok() != Some(source_parent))
    {
        return Err("Selected occurrences must be siblings under the same parent".into());
    }
    let initial = displayed_siblings(
        project.get_pool(),
        project.component_source(),
        source_parent,
    )
    .map_err(|e| e.to_string())?;
    let selected: Vec<_> = initial
        .iter()
        .enumerate()
        .filter(|(i, _)| {
            targets
                .iter()
                .any(|t| sibling_matches_target(&initial, *i, t))
        })
        .map(|(_, s)| s.clone())
        .collect();
    if selected.len() != targets.iter().collect::<HashSet<_>>().len() {
        return Err("A selected occurrence no longer exists".into());
    }
    if selected.len() != targets.len() {
        return Err("The same occurrence was selected more than once".into());
    }
    let same_parent = source_parent == destination_parent;
    let materialized = project.materialize().map_err(|e| e.to_string())?;
    let destination = project
        .get_pool()
        .object_by_id(object_id(destination_parent).map_err(|e| e.to_string())?)
        .ok_or("Missing destination parent")?;
    if object_refs(destination).is_none() {
        return Err("Destination does not support positioned children".into());
    }
    let mut size = match destination {
        Object::DataMask(_) | Object::AlarmMask(_) => (project.mask_size, project.mask_size),
        Object::Key(_)
        | Object::WorkingSet(_)
        | Object::AuxiliaryFunctionType1(_)
        | Object::AuxiliaryInputType1(_)
        | Object::AuxiliaryFunctionType2(_)
        | Object::AuxiliaryInputType2(_) => project.soft_key_size(),
        _ => project.get_pool().content_size(destination),
    };
    if matches!(destination, Object::Button(_)) {
        let insets = (crate::button_geometry::object_face_inset(destination, project.vt_version)
            * 2.0) as u16;
        size.0 = size.0.saturating_sub(insets);
        size.1 = size.1.saturating_sub(insets);
    }
    let mut moved = Vec::new();
    for sibling in &selected {
        let id = displayed_child_id(sibling, &materialized, project.component_source())?;
        let child = materialized
            .pool
            .object_by_id(object_id(id).map_err(|e| e.to_string())?)
            .ok_or("Missing selected object")?;
        let child_type = materialized
            .pool
            .object_by_id(object_id(id).map_err(|e| e.to_string())?)
            .ok_or("Missing selected object")?
            .object_type();
        if !crate::allowed_object_relationships::get_allowed_child_refs(
            destination.object_type(),
            project.vt_version,
        )
        .contains(&child_type)
        {
            return Err(format!(
                "{:?} cannot contain {child_type:?} for VT {:?}",
                destination.object_type(),
                project.vt_version
            ));
        }
        let (old_x, old_y) = displayed_position(sibling);
        let x = if same_parent {
            i32::from(old_x)
        } else {
            i32::from(old_x) + coordinate_delta.0
        };
        let y = if same_parent {
            i32::from(old_y)
        } else {
            i32::from(old_y) + coordinate_delta.1
        };
        let (child_width, child_height) =
            occurrence_size(&materialized.pool, child, project.soft_key_size());
        if !same_parent
            && (x < 0
                || y < 0
                || x + child_width > i32::from(size.0)
                || y + child_height > i32::from(size.1))
        {
            return Err(format!(
                "Object {} does not fit inside the destination",
                project.get_object_info(child).get_name(child)
            ));
        }
        moved.push(match sibling {
            DisplayedSibling::Primitive(p) => DisplayedSibling::Primitive(ChildPlacement {
                x: i16::try_from(x).map_err(|_| "Destination x position is out of range")?,
                y: i16::try_from(y).map_err(|_| "Destination y position is out of range")?,
                ..p.clone()
            }),
            DisplayedSibling::Component(instance) => {
                let mut copy = instance.clone();
                copy.x = x;
                copy.y = y;
                copy.parent = EditorParentRef::Primitive(destination_parent);
                DisplayedSibling::Component(copy)
            }
        });
    }
    let version = project.vt_version;
    let selected_targets = targets.to_vec();
    let mut output = Vec::new();
    project
        .edit_source(
            "Move sibling placements",
            |pool, source, _graphics, _info| {
                let mut source_siblings = displayed_siblings(pool, source, source_parent)?;
                let extracted: Vec<_> = source_siblings
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| {
                        selected_targets
                            .iter()
                            .any(|t| sibling_matches_target(&source_siblings, *i, t))
                    })
                    .map(|(_, s)| s.clone())
                    .collect();
                if extracted.len() != selected_targets.len() {
                    return Err(invalid_placement());
                }
                let source_before = source_siblings.clone();
                source_siblings = source_before
                    .iter()
                    .enumerate()
                    .filter(|(i, _)| {
                        !selected_targets
                            .iter()
                            .any(|t| sibling_matches_target(&source_before, *i, t))
                    })
                    .map(|(_, s)| s.clone())
                    .collect();
                if same_parent {
                    let before = insertion_index.unwrap_or(initial.len()).min(initial.len());
                    let removed_before = initial
                        .iter()
                        .take(before)
                        .enumerate()
                        .filter(|(i, _)| {
                            selected_targets
                                .iter()
                                .any(|t| sibling_matches_target(&initial, *i, t))
                        })
                        .count();
                    let slot = before
                        .saturating_sub(removed_before)
                        .min(source_siblings.len());
                    for (offset, sibling) in moved.iter().cloned().enumerate() {
                        source_siblings.insert(slot + offset, sibling);
                    }
                    if same_displayed_siblings(&source_siblings, &initial) {
                        output = selected_targets.clone();
                        return Ok(());
                    }
                    set_displayed_siblings(
                        pool,
                        source,
                        source_parent,
                        source_siblings.clone(),
                        version,
                    )?;
                    output = targets_for_block(&source_siblings, slot, moved.len(), source_parent);
                } else {
                    let mut dest = displayed_siblings(pool, source, destination_parent)?;
                    let slot = insertion_index.unwrap_or(dest.len()).min(dest.len());
                    for (offset, sibling) in moved.iter().cloned().enumerate() {
                        dest.insert(slot + offset, sibling);
                    }
                    set_displayed_siblings(pool, source, source_parent, source_siblings, version)?;
                    set_displayed_siblings(
                        pool,
                        source,
                        destination_parent,
                        dest.clone(),
                        version,
                    )?;
                    output = targets_for_block(&dest, slot, moved.len(), destination_parent);
                }
                Ok(())
            },
        )
        .map_err(|error| error.to_string())?;
    Ok(output)
}

/// Check a block move using the same validation and fit rules as execution,
/// without changing the supplied project.
pub fn validate_move_placements(
    project: &EditorProject,
    targets: &[PlacementTarget],
    destination_parent: u16,
    insertion_index: Option<usize>,
    coordinate_delta: (i32, i32),
) -> Result<(), String> {
    let mut preview = project.canvas_preview_copy();
    move_placements(
        &mut preview,
        targets,
        destination_parent,
        insertion_index,
        coordinate_delta,
    )
    .map(|_| ())
}

fn targets_for_block(
    siblings: &[DisplayedSibling],
    slot: usize,
    count: usize,
    parent_id: u16,
) -> Vec<PlacementTarget> {
    let mut primitive_index = 0usize;
    let mut output = Vec::new();
    for (display_index, sibling) in siblings.iter().enumerate() {
        if display_index >= slot && display_index < slot + count {
            output.push(match sibling {
                DisplayedSibling::Primitive(_) => PlacementTarget::Primitive {
                    parent_id,
                    child_index: primitive_index,
                },
                DisplayedSibling::Component(instance) => PlacementTarget::Component {
                    instance_id: instance.id,
                },
            });
        }
        if matches!(sibling, DisplayedSibling::Primitive(_)) {
            primitive_index += 1;
        }
    }
    output
}

impl PlacementTarget {
    fn parent_id(&self, project: &EditorProject) -> Result<u16, String> {
        match self {
            Self::Primitive {
                parent_id,
                child_index,
            } => {
                if *child_index
                    >= child_placements(project.get_pool(), *parent_id)
                        .map_err(|e| e.to_string())?
                        .len()
                {
                    return Err("Selected occurrence no longer exists".into());
                }
                Ok(*parent_id)
            }
            Self::Component { instance_id } => project
                .component_source()
                .instances
                .iter()
                .find(|i| i.id == *instance_id)
                .map(|i| match i.parent {
                    EditorParentRef::Primitive(id) => id,
                })
                .ok_or_else(|| "Selected component no longer exists".into()),
        }
    }
}

fn sibling_matches_target(
    siblings: &[DisplayedSibling],
    display_index: usize,
    target: &PlacementTarget,
) -> bool {
    let Some(sibling) = siblings.get(display_index) else {
        return false;
    };
    match (sibling, target) {
        (DisplayedSibling::Primitive(_), PlacementTarget::Primitive { child_index, .. }) => {
            siblings[..display_index]
                .iter()
                .filter(|s| matches!(s, DisplayedSibling::Primitive(_)))
                .count()
                == *child_index
        }
        (DisplayedSibling::Component(i), PlacementTarget::Component { instance_id }) => {
            i.id == *instance_id
        }
        _ => false,
    }
}

fn displayed_position(sibling: &DisplayedSibling) -> (i16, i16) {
    match sibling {
        DisplayedSibling::Primitive(p) => (p.x, p.y),
        DisplayedSibling::Component(i) => (i.x as i16, i.y as i16),
    }
}

fn occurrence_size(
    pool: &ObjectPool,
    object: &ag_iso_stack::object_pool::object::Object,
    soft_key_size: (u16, u16),
) -> (i32, i32) {
    let bounds = crate::layout_containment::bounds_for_object_with_soft_key_size(
        pool,
        object,
        Some(soft_key_size),
    );
    (bounds.width, bounds.height)
}

fn displayed_child_id(
    sibling: &DisplayedSibling,
    materialized: &crate::components::MaterializedProject,
    source: &ComponentSource,
) -> Result<u16, String> {
    match sibling {
        DisplayedSibling::Primitive(p) => {
            target_id(&p.child).ok_or_else(|| "Selected child has no object ID".into())
        }
        DisplayedSibling::Component(instance) => {
            let definition = source
                .definitions
                .get(&instance.component_id)
                .ok_or("Missing component definition")?;
            materialized
                .source_map
                .by_component_node
                .get(&(instance.id, definition.root_node))
                .copied()
                .ok_or_else(|| "Missing component root".into())
        }
    }
}

fn same_displayed_siblings(left: &[DisplayedSibling], right: &[DisplayedSibling]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    left.iter().zip(right).all(|(a, b)| match (a, b) {
        (DisplayedSibling::Primitive(a), DisplayedSibling::Primitive(b)) => {
            a.child == b.child && a.x == b.x && a.y == b.y
        }
        (DisplayedSibling::Component(a), DisplayedSibling::Component(b)) => {
            a.id == b.id && a.x == b.x && a.y == b.y
        }
        _ => false,
    })
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
            if matches!(
                pool.object_by_id(object_id(root_id)?),
                Some(ag_iso_stack::object_pool::object::Object::PictureGraphic(_))
            ) {
                // A picture selected as the duplicate target is itself the
                // object being copied. Pictures reached below a container
                // remain shared image resources.
                graph_ids.push(root_id);
            } else {
                collect_positioned_subtree(pool, root_id, &mut HashSet::new(), &mut graph_ids)?;
            }
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
    // Image data is a shared resource when reached through a container.
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

#[cfg(test)]
mod tests {
    use super::{
        apply, group_placements, hierarchy_path_after_removals, move_placements, position,
        CanvasEdit, PlacementTarget,
    };
    use crate::{default_object, EditorProject};
    use ag_iso_stack::object_pool::{
        object::{Object, PictureGraphic},
        object_attributes::{LineDirection, ObjectRef, Point},
        ObjectId, ObjectPool, ObjectType,
    };

    fn with_id(object_type: ObjectType, id: u16) -> Object {
        let mut object = default_object(object_type);
        object.mut_id().set_value(id).unwrap();
        object
    }

    fn add_child(pool: &mut ObjectPool, parent_id: u16, child_id: u16) {
        let parent = pool
            .object_mut_by_id(ObjectId::new(parent_id).unwrap())
            .unwrap();
        let references = match parent {
            Object::DataMask(mask) => &mut mask.object_refs,
            Object::Container(container) => &mut container.object_refs,
            _ => panic!("expected a positioned parent"),
        };
        references.push(ObjectRef {
            id: ObjectId::new(child_id).unwrap(),
            offset: Point { x: 0, y: 0 },
        });
    }

    fn picture(project: &EditorProject, id: u16) -> PictureGraphic {
        match project
            .get_pool()
            .object_by_id(ObjectId::new(id).unwrap())
            .unwrap()
        {
            Object::PictureGraphic(picture) => picture.clone(),
            _ => panic!("expected a PictureGraphic"),
        }
    }

    fn mask_child_count(project: &EditorProject) -> usize {
        match project
            .get_pool()
            .object_by_id(ObjectId::new(1).unwrap())
            .unwrap()
        {
            Object::DataMask(mask) => mask.object_refs.len(),
            _ => unreachable!(),
        }
    }

    #[test]
    fn duplicate_picture_copies_object_while_container_duplicate_shares_picture() {
        let mut pool = ObjectPool::new();
        pool.add(with_id(ObjectType::DataMask, 1));
        pool.add(with_id(ObjectType::PictureGraphic, 2));
        pool.add(with_id(ObjectType::Container, 3));
        if let Some(Object::Container(container)) = pool.object_mut_by_id(ObjectId::new(3).unwrap())
        {
            container.width = 100;
            container.height = 60;
        }
        if let Some(Object::PictureGraphic(picture)) =
            pool.object_mut_by_id(ObjectId::new(2).unwrap())
        {
            picture.width = 8;
            picture.actual_width = 8;
            picture.actual_height = 1;
            picture.data = vec![0xA5];
        }
        add_child(&mut pool, 1, 2);
        add_child(&mut pool, 1, 3);
        add_child(&mut pool, 3, 2);
        let mut project = EditorProject::from(pool);
        project.mask_size = 500;

        let copied_picture = apply(
            &mut project,
            "Duplicate picture",
            &[CanvasEdit::Duplicate {
                target: PlacementTarget::Primitive {
                    parent_id: 1,
                    child_index: 0,
                },
                x: 10,
                y: 20,
            }],
        )
        .unwrap();
        let PlacementTarget::Primitive {
            parent_id: 1,
            child_index,
        } = copied_picture[0]
        else {
            panic!("expected duplicated primitive placement");
        };
        let duplicate_id = match &project
            .get_pool()
            .object_by_id(ObjectId::new(1).unwrap())
            .unwrap()
        {
            Object::DataMask(mask) => mask.object_refs[child_index].id.value(),
            _ => unreachable!(),
        };
        assert_ne!(duplicate_id, 2);
        assert_eq!(
            picture(&project, duplicate_id).data,
            picture(&project, 2).data
        );
        assert_eq!(
            picture(&project, duplicate_id).width,
            picture(&project, 2).width
        );
        assert_eq!(mask_child_count(&project), 3);
        assert!(project.undo_operation());
        assert!(project
            .get_pool()
            .object_by_id(ObjectId::new(duplicate_id).unwrap())
            .is_none());
        assert_eq!(mask_child_count(&project), 2);
        assert!(project.redo_operation());
        assert_eq!(
            picture(&project, duplicate_id).data,
            picture(&project, 2).data
        );
        assert_eq!(mask_child_count(&project), 3);

        let copied_container = apply(
            &mut project,
            "Duplicate container",
            &[CanvasEdit::Duplicate {
                target: PlacementTarget::Primitive {
                    parent_id: 1,
                    child_index: 1,
                },
                x: 30,
                y: 40,
            }],
        )
        .unwrap();
        let PlacementTarget::Primitive {
            parent_id: 1,
            child_index,
        } = copied_container[0]
        else {
            panic!("expected duplicated primitive placement");
        };
        let container_id = match &project
            .get_pool()
            .object_by_id(ObjectId::new(1).unwrap())
            .unwrap()
        {
            Object::DataMask(mask) => mask.object_refs[child_index].id.value(),
            _ => unreachable!(),
        };
        match project
            .get_pool()
            .object_by_id(ObjectId::new(container_id).unwrap())
            .unwrap()
        {
            Object::Container(container) => {
                assert_eq!(container.object_refs[0].id.value(), 2);
            }
            _ => panic!("expected duplicated container"),
        }
    }

    #[test]
    fn shape_edits_are_shared_and_line_geometry_is_one_undoable_change() {
        let mut pool = ObjectPool::new();
        pool.add(with_id(ObjectType::DataMask, 1));
        pool.add(with_id(ObjectType::OutputPolygon, 2));
        pool.add(with_id(ObjectType::OutputLine, 3));
        pool.add(with_id(ObjectType::LineAttributes, 4));
        if let Some(Object::OutputPolygon(polygon)) =
            pool.object_mut_by_id(ObjectId::new(2).unwrap())
        {
            polygon.width = 80;
            polygon.height = 60;
            polygon.points = vec![
                Point { x: 2, y: 3 },
                Point { x: 20, y: 3 },
                Point { x: 20, y: 20 },
            ];
        }
        if let Some(Object::OutputLine(line)) = pool.object_mut_by_id(ObjectId::new(3).unwrap()) {
            line.line_attributes = ObjectId::new(4).unwrap().into();
            line.width = 30;
            line.height = 20;
        }
        if let Some(Object::LineAttributes(line)) = pool.object_mut_by_id(ObjectId::new(4).unwrap())
        {
            line.line_width = 4;
        }
        add_child(&mut pool, 1, 2);
        add_child(&mut pool, 1, 2);
        add_child(&mut pool, 1, 3);
        let mut project = EditorProject::from(pool);
        project.mask_size = 500;
        let polygon_target = PlacementTarget::Primitive {
            parent_id: 1,
            child_index: 0,
        };
        let selected = apply(
            &mut project,
            "Move polygon point",
            &[CanvasEdit::SetPolygonPoint {
                object_id: 2,
                target: polygon_target.clone(),
                index: 1,
                x: 25,
                y: 12,
            }],
        )
        .unwrap();
        assert_eq!(selected, vec![polygon_target]);
        let polygon_point = |project: &EditorProject| match project
            .get_pool()
            .object_by_id(ObjectId::new(2).unwrap())
            .unwrap()
        {
            Object::OutputPolygon(polygon) => polygon.points[1],
            _ => unreachable!(),
        };
        assert_eq!(polygon_point(&project), Point { x: 25, y: 12 });
        assert!(project.undo_operation());
        assert_eq!(polygon_point(&project), Point { x: 20, y: 3 });
        assert!(project.redo_operation());
        assert_eq!(polygon_point(&project), Point { x: 25, y: 12 });

        let line_target = PlacementTarget::Primitive {
            parent_id: 1,
            child_index: 2,
        };
        apply(
            &mut project,
            "Move line endpoint",
            &[CanvasEdit::EditLineEndpoint {
                object_id: 3,
                target: line_target.clone(),
                x: 20,
                y: 30,
                width: 50,
                height: 40,
                direction: LineDirection::BottomLeftToTopRight,
            }],
        )
        .unwrap();
        assert_eq!(position(&project, &line_target), Some((20, 30)));
        let line_geometry = |project: &EditorProject| match project
            .get_pool()
            .object_by_id(ObjectId::new(3).unwrap())
            .unwrap()
        {
            Object::OutputLine(line) => (line.width, line.height, line.line_direction),
            _ => unreachable!(),
        };
        assert_eq!(
            line_geometry(&project),
            (50, 40, LineDirection::BottomLeftToTopRight)
        );
        assert!(project.undo_operation());
        assert_eq!(position(&project, &line_target), Some((0, 0)));
        assert_eq!(
            line_geometry(&project),
            (30, 20, LineDirection::TopLeftToBottomRight)
        );
        assert!(project.redo_operation());
        assert_eq!(position(&project, &line_target), Some((20, 30)));
        assert_eq!(
            line_geometry(&project),
            (50, 40, LineDirection::BottomLeftToTopRight)
        );
    }

    #[test]
    fn grouping_preserves_selected_positions_and_is_one_undoable_change() {
        let mut pool = ObjectPool::new();
        pool.add(with_id(ObjectType::DataMask, 1));
        for id in 2..=3 {
            let mut rect = with_id(ObjectType::OutputRectangle, id);
            if let Object::OutputRectangle(rect) = &mut rect {
                rect.width = 12;
                rect.height = 8;
            }
            pool.add(rect);
            add_child(&mut pool, 1, id);
        }
        if let Some(Object::DataMask(mask)) = pool.object_mut_by_id(ObjectId::new(1).unwrap()) {
            mask.object_refs[0].offset = Point { x: 20, y: 30 };
            mask.object_refs[1].offset = Point { x: 60, y: 40 };
        }
        let mut project = EditorProject::from(pool);
        project.mask_size = 500;
        let grouped = group_placements(
            &mut project,
            &[
                PlacementTarget::Primitive {
                    parent_id: 1,
                    child_index: 0,
                },
                PlacementTarget::Primitive {
                    parent_id: 1,
                    child_index: 1,
                },
            ],
            ObjectType::Container,
            "Grouped",
        )
        .unwrap();
        let PlacementTarget::Primitive {
            parent_id: 1,
            child_index,
        } = grouped
        else {
            panic!("expected a primitive group occurrence");
        };
        let group_id = match project
            .get_pool()
            .object_by_id(ObjectId::new(1).unwrap())
            .unwrap()
        {
            Object::DataMask(mask) => mask.object_refs[child_index].id.value(),
            _ => unreachable!(),
        };
        assert_eq!(
            position(
                &project,
                &PlacementTarget::Primitive {
                    parent_id: 1,
                    child_index
                }
            ),
            Some((20, 30))
        );
        match project
            .get_pool()
            .object_by_id(ObjectId::new(group_id).unwrap())
            .unwrap()
        {
            Object::Container(container) => {
                assert_eq!((container.width, container.height), (52, 18));
                assert_eq!(
                    container
                        .object_refs
                        .iter()
                        .map(|r| (r.id.value(), r.offset.x, r.offset.y))
                        .collect::<Vec<_>>(),
                    vec![(2, 0, 0), (3, 40, 10)]
                );
            }
            _ => panic!("expected a Container"),
        }
        assert!(project.undo_operation());
        assert_eq!(mask_child_count(&project), 2);
        assert!(project.redo_operation());
        assert_eq!(mask_child_count(&project), 1);
    }

    #[test]
    fn moving_noncontiguous_siblings_reorders_them_as_a_block() {
        let mut pool = ObjectPool::new();
        pool.add(with_id(ObjectType::DataMask, 1));
        for id in 2..=4 {
            pool.add(with_id(ObjectType::OutputRectangle, id));
            add_child(&mut pool, 1, id);
        }
        let mut project = EditorProject::from(pool);
        project.mask_size = 500;
        move_placements(
            &mut project,
            &[
                PlacementTarget::Primitive {
                    parent_id: 1,
                    child_index: 0,
                },
                PlacementTarget::Primitive {
                    parent_id: 1,
                    child_index: 2,
                },
            ],
            1,
            Some(2),
            (0, 0),
        )
        .unwrap();
        let ids = match project
            .get_pool()
            .object_by_id(ObjectId::new(1).unwrap())
            .unwrap()
        {
            Object::DataMask(mask) => mask
                .object_refs
                .iter()
                .map(|r| r.id.value())
                .collect::<Vec<_>>(),
            _ => unreachable!(),
        };
        assert_eq!(ids, vec![3, 2, 4]);
        assert!(project.undo_operation());
        let restored = match project
            .get_pool()
            .object_by_id(ObjectId::new(1).unwrap())
            .unwrap()
        {
            Object::DataMask(mask) => mask
                .object_refs
                .iter()
                .map(|r| r.id.value())
                .collect::<Vec<_>>(),
            _ => unreachable!(),
        };
        assert_eq!(restored, vec![2, 3, 4]);
    }

    #[test]
    fn one_occurrence_can_be_grouped_under_a_button_with_content_inset() {
        let mut pool = ObjectPool::new();
        pool.add(with_id(ObjectType::DataMask, 1));
        let mut rect = with_id(ObjectType::OutputRectangle, 2);
        if let Object::OutputRectangle(rect) = &mut rect {
            rect.width = 12;
            rect.height = 8;
        }
        pool.add(rect);
        add_child(&mut pool, 1, 2);
        if let Some(Object::DataMask(mask)) = pool.object_mut_by_id(ObjectId::new(1).unwrap()) {
            mask.object_refs[0].offset = Point { x: 10, y: 12 };
        }
        let mut project = EditorProject::from(pool);
        project.mask_size = 500;
        let grouped = group_placements(
            &mut project,
            &[PlacementTarget::Primitive {
                parent_id: 1,
                child_index: 0,
            }],
            ObjectType::Button,
            "Single child",
        )
        .unwrap();
        let PlacementTarget::Primitive {
            parent_id: 1,
            child_index,
        } = grouped
        else {
            panic!("expected a primitive group occurrence");
        };
        let group_id = match project
            .get_pool()
            .object_by_id(ObjectId::new(1).unwrap())
            .unwrap()
        {
            Object::DataMask(mask) => mask.object_refs[child_index].id.value(),
            _ => unreachable!(),
        };
        assert_eq!(
            position(
                &project,
                &PlacementTarget::Primitive {
                    parent_id: 1,
                    child_index
                }
            ),
            Some((6, 8))
        );
        match project
            .get_pool()
            .object_by_id(ObjectId::new(group_id).unwrap())
            .unwrap()
        {
            Object::Button(button) => {
                assert_eq!((button.width, button.height), (20, 16));
                assert_eq!(
                    (
                        button.object_refs[0].offset.x,
                        button.object_refs[0].offset.y
                    ),
                    (0, 0)
                );
            }
            _ => panic!("expected a Button"),
        }
        assert_eq!(
            crate::canvas::edits::hierarchy_child_origin(&project, 1, &[0]).unwrap(),
            egui::pos2(10.0, 12.0)
        );
    }

    #[test]
    fn compatible_animation_can_group_positioned_children() {
        let mut pool = ObjectPool::new();
        pool.add(with_id(ObjectType::DataMask, 1));
        let mut rect = with_id(ObjectType::OutputRectangle, 2);
        if let Object::OutputRectangle(rect) = &mut rect {
            rect.width = 12;
            rect.height = 8;
        }
        pool.add(rect);
        add_child(&mut pool, 1, 2);
        let mut project = EditorProject::from(pool);
        project.mask_size = 500;
        let candidates = super::grouping_candidates(
            &project,
            &[PlacementTarget::Primitive {
                parent_id: 1,
                child_index: 0,
            }],
        );
        assert!(candidates
            .iter()
            .any(|candidate| candidate.object_type == ObjectType::Animation));
        let grouped = group_placements(
            &mut project,
            &[PlacementTarget::Primitive {
                parent_id: 1,
                child_index: 0,
            }],
            ObjectType::Animation,
            "Animation parent",
        )
        .unwrap();
        let PlacementTarget::Primitive {
            parent_id: 1,
            child_index,
        } = grouped
        else {
            panic!("expected a primitive group occurrence");
        };
        let group_id = match project
            .get_pool()
            .object_by_id(ObjectId::new(1).unwrap())
            .unwrap()
        {
            Object::DataMask(mask) => mask.object_refs[child_index].id.value(),
            _ => unreachable!(),
        };
        match project
            .get_pool()
            .object_by_id(ObjectId::new(group_id).unwrap())
            .unwrap()
        {
            Object::Animation(animation) => {
                assert_eq!((animation.width, animation.height), (12, 8));
                assert_eq!(
                    (animation.first_child_index, animation.last_child_index),
                    (0, 0)
                );
                assert_eq!(animation.object_refs[0].id.value(), 2);
            }
            _ => panic!("expected an Animation"),
        }
    }

    #[test]
    fn hierarchy_destination_path_accounts_for_multiple_removed_siblings() {
        let mut pool = ObjectPool::new();
        pool.add(with_id(ObjectType::DataMask, 1));
        for id in 2..=5 {
            pool.add(with_id(ObjectType::Container, id));
        }
        add_child(&mut pool, 1, 2);
        add_child(&mut pool, 1, 3);
        add_child(&mut pool, 1, 4);
        add_child(&mut pool, 4, 5);
        let project = EditorProject::from(pool);
        let adjusted = hierarchy_path_after_removals(
            &project,
            1,
            &[0, 2, 0],
            &[
                PlacementTarget::Primitive {
                    parent_id: 1,
                    child_index: 0,
                },
                PlacementTarget::Primitive {
                    parent_id: 1,
                    child_index: 1,
                },
            ],
        )
        .unwrap();
        assert_eq!(adjusted, vec![0, 0, 0]);
    }

    #[test]
    fn grouping_candidates_reject_designators_that_are_too_small_for_the_selection() {
        let mut pool = ObjectPool::new();
        pool.add(with_id(ObjectType::DataMask, 1));
        let mut rect = with_id(ObjectType::OutputRectangle, 2);
        if let Object::OutputRectangle(rect) = &mut rect {
            rect.width = 150;
            rect.height = 90;
        }
        pool.add(rect);
        add_child(&mut pool, 1, 2);
        let mut project = EditorProject::from(pool);
        project.mask_size = 500;
        project.set_design_dimensions(500, (100, 80)).unwrap();

        let candidates = super::grouping_candidates(
            &project,
            &[PlacementTarget::Primitive {
                parent_id: 1,
                child_index: 0,
            }],
        );
        for object_type in [ObjectType::WorkingSet, ObjectType::AuxiliaryFunctionType2] {
            let candidate = candidates
                .iter()
                .find(|candidate| candidate.object_type == object_type)
                .expect("the compatible designator type should be offered");
            assert!(!candidate.enabled);
            assert!(candidate.reason.as_deref().unwrap().contains("do not fit"));
        }
    }

    #[test]
    fn grouping_designator_must_fit_inside_the_current_mask() {
        let mut pool = ObjectPool::new();
        pool.add(with_id(ObjectType::DataMask, 1));
        let mut rect = with_id(ObjectType::OutputRectangle, 2);
        if let Object::OutputRectangle(rect) = &mut rect {
            rect.width = 10;
            rect.height = 10;
        }
        pool.add(rect);
        add_child(&mut pool, 1, 2);
        if let Some(Object::DataMask(mask)) = pool.object_mut_by_id(ObjectId::new(1).unwrap()) {
            mask.object_refs[0].offset = Point { x: 490, y: 0 };
        }
        let mut project = EditorProject::from(pool);
        project.set_design_dimensions(500, (100, 80)).unwrap();
        let target = PlacementTarget::Primitive {
            parent_id: 1,
            child_index: 0,
        };

        let mut interior = project.clone();
        apply(
            &mut interior,
            "Move selection inside mask",
            &[CanvasEdit::Move {
                target: target.clone(),
                x: 0,
                y: 0,
            }],
        )
        .unwrap();
        let interior_candidates =
            super::grouping_candidates(&interior, std::slice::from_ref(&target));
        let candidates = super::grouping_candidates(&project, std::slice::from_ref(&target));
        for object_type in [ObjectType::WorkingSet, ObjectType::AuxiliaryFunctionType2] {
            assert!(
                interior_candidates
                    .iter()
                    .find(|candidate| candidate.object_type == object_type)
                    .expect("the compatible designator type should be offered")
                    .enabled
            );
            let candidate = candidates
                .iter()
                .find(|candidate| candidate.object_type == object_type)
                .expect("the compatible designator type should be offered");
            assert!(!candidate.enabled);
        }

        let before_pool = project.get_pool().clone();
        let before_source = project.component_source().clone();
        assert!(group_placements(
            &mut project,
            &[target],
            ObjectType::WorkingSet,
            "Off-screen designator",
        )
        .is_err());
        assert_eq!(project.get_pool(), &before_pool);
        assert_eq!(project.component_source(), &before_source);
    }
}
