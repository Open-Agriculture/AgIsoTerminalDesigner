//! Shared geometry checks for positioned object references.
//!
//! Bounds are in pool coordinates and describe the nominal, unclipped extent
//! used by the canvas. Placement validation deliberately visits every mask so
//! shared objects cannot be resized out of bounds in an inactive mask.

use ag_iso_stack::object_pool::{
    object::Object, object_attributes::ObjectRef, vt_version::VtVersion, ObjectPool,
};
use std::{
    collections::{HashMap, HashSet},
    fmt,
};

pub(crate) const OBJECT_OUTSIDE_PARENT: &str = "OBJECT_OUTSIDE_PARENT";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Bounds {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct OriginRange {
    pub min_x: i32,
    pub max_x: i32,
    pub min_y: i32,
    pub max_y: i32,
}

/// Error returned when a placement exceeds its parent's content bounds.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct ContainmentError {
    pub child_id: u16,
    pub parent_id: u16,
    pub message: String,
}

impl ContainmentError {
    pub(crate) fn code(&self) -> &'static str {
        OBJECT_OUTSIDE_PARENT
    }
}

impl fmt::Display for ContainmentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

/// Returns the nominal size at an origin of zero, following pointer aliases.
/// Sized objects use their object-pool dimensions; pictures therefore retain
/// the height computed from their encoded aspect ratio.
/// Fixed designators use the configured Soft Key size when provided.
pub(crate) fn bounds_for_object_with_soft_key_size(
    pool: &ObjectPool,
    object: &Object,
    soft_key_size: Option<(u16, u16)>,
) -> Bounds {
    bounds_for_object_inner(pool, object, soft_key_size, &mut HashSet::new())
}

fn bounds_for_object_inner(
    pool: &ObjectPool,
    object: &Object,
    soft_key_size: Option<(u16, u16)>,
    visiting: &mut HashSet<u16>,
) -> Bounds {
    let Some(object) = resolve_pointer(pool, object) else {
        return Bounds::default();
    };
    let (width, height) = match (object, soft_key_size) {
        (object, Some((width, height))) if uses_soft_key_dimensions(object) => {
            (width as i32, height as i32)
        }
        (object, _) => object
            .as_sized_object()
            .map(|sized| (sized.width() as i32, sized.height() as i32))
            .unwrap_or_else(|| dynamic_size(pool, object, soft_key_size, visiting)),
    };
    Bounds {
        x: 0,
        y: 0,
        width,
        height,
    }
}

/// Returns the content area of an object within its nominal outer rectangle.
/// Bordered Buttons use the preview's four-pixel face inset; VT4+
/// borderless Buttons use their full outer area.
pub(crate) fn content_bounds(object: &Object, outer: Bounds, version: VtVersion) -> Bounds {
    if matches!(object, Object::Button(_)) {
        let inset = crate::button_geometry::object_face_inset(object, version) as i32;
        Bounds {
            x: outer.x + inset,
            y: outer.y + inset,
            width: (outer.width - 2 * inset).max(0),
            height: (outer.height - 2 * inset).max(0),
        }
    } else {
        outer
    }
}

/// Legal absolute origins for one child rectangle. When `allow_existing_overflow`
/// is true, each edge may retain its current overflow, but cannot increase it.
pub(crate) fn allowed_origin_range(
    parent: Bounds,
    child_size: (i32, i32),
    current_origin: (i32, i32),
    allow_existing_overflow: bool,
) -> OriginRange {
    let (width, height) = child_size;
    let overflow = |start: i32, size: i32, p_start: i32, p_size: i32| {
        let end = start.saturating_add(size);
        (
            (p_start - start).max(0),
            (end - p_start.saturating_add(p_size)).max(0),
        )
    };
    let (left, right) = if allow_existing_overflow {
        overflow(current_origin.0, width, parent.x, parent.width)
    } else {
        (0, 0)
    };
    let (top, bottom) = if allow_existing_overflow {
        overflow(current_origin.1, height, parent.y, parent.height)
    } else {
        (0, 0)
    };
    OriginRange {
        min_x: parent.x - left,
        max_x: parent
            .x
            .saturating_add(parent.width)
            .saturating_add(right)
            .saturating_sub(width),
        min_y: parent.y - top,
        max_y: parent
            .y
            .saturating_add(parent.height)
            .saturating_add(bottom)
            .saturating_sub(height),
    }
}

/// Checks that an edit introduces no overflow and does not increase any
/// existing edge overflow for a placement. Every mask is traversed, including
/// inactive masks, plus containers with no parent placement.
/// Uses separate old and candidate dimensions for
/// every Key's child content area.
pub(crate) fn validate_change_with_soft_key_size(
    before: &ObjectPool,
    after: &ObjectPool,
    before_mask: u16,
    after_mask: u16,
    before_soft_key_size: (u16, u16),
    after_soft_key_size: (u16, u16),
    version: VtVersion,
) -> Result<(), ContainmentError> {
    validate_change_with_id_changes_and_soft_key_size(
        before,
        after,
        before_mask,
        after_mask,
        &[],
        Some((before_soft_key_size, after_soft_key_size)),
        version,
    )
}

/// Validates a change while preserving placement identity across object ID
/// renames. Renames are applied to baseline placement identities in order.
pub(crate) fn validate_change_with_id_changes_and_soft_key_size(
    before: &ObjectPool,
    after: &ObjectPool,
    before_mask: u16,
    after_mask: u16,
    id_changes: &[(u16, u16)],
    soft_key_sizes: Option<((u16, u16), (u16, u16))>,
    version: VtVersion,
) -> Result<(), ContainmentError> {
    let before_placements = remap_placement_ids(
        placements(
            before,
            before_mask,
            soft_key_sizes.map(|sizes| sizes.0),
            version,
        ),
        id_changes,
    );
    let after_placements = placements(
        after,
        after_mask,
        soft_key_sizes.map(|sizes| sizes.1),
        version,
    );
    for (key, current) in after_placements {
        let previous = before_placements
            .get(&key)
            .map(Vec::as_slice)
            .unwrap_or(&[]);
        if let Some(index) = unmatched_overflow(&current, previous) {
            let placement = &current[index];
            let edge = overflow_edge(placement.overflow, previous);
            return Err(ContainmentError {
                child_id: placement.child_id,
                parent_id: placement.parent_id,
                message: format!(
                    "Object {} is outside parent {} on the {edge} edge at ({}, {}).",
                    placement.child_id, placement.parent_id, placement.x, placement.y
                ),
            });
        }
    }
    Ok(())
}

fn remap_placement_ids(
    placements: HashMap<PlacementKey, Vec<Placement>>,
    id_changes: &[(u16, u16)],
) -> HashMap<PlacementKey, Vec<Placement>> {
    let mut remapped = HashMap::new();
    for (mut key, mut values) in placements {
        key.parent_id = remap_id(key.parent_id, id_changes);
        key.child_id = remap_id(key.child_id, id_changes);
        for (parent_id, child_id) in &mut key.context {
            *parent_id = remap_id(*parent_id, id_changes);
            *child_id = remap_id(*child_id, id_changes);
        }
        for placement in &mut values {
            placement.parent_id = remap_id(placement.parent_id, id_changes);
            placement.child_id = remap_id(placement.child_id, id_changes);
        }
        remapped.entry(key).or_insert_with(Vec::new).extend(values);
    }
    remapped
}

fn remap_id(id: u16, id_changes: &[(u16, u16)]) -> u16 {
    id_changes
        .iter()
        .find(|(old, _)| *old == id)
        .map_or(id, |(_, new)| *new)
}

/// Strictly validates references introduced by the current edit, even when a
/// prior occurrence of the same child had grandfathered overflow. The caller
/// supplies IDs and list indices from the operation that added those refs.
pub(crate) fn validate_new_placements_with_soft_key_size(
    after: &ObjectPool,
    mask_size: u16,
    new_refs: &HashSet<(u16, usize)>,
    soft_key_size: Option<(u16, u16)>,
    version: VtVersion,
) -> Result<(), ContainmentError> {
    for placements in placements(after, mask_size, soft_key_size, version).into_values() {
        for placement in placements {
            if new_refs.contains(&(placement.parent_id, placement.child_index))
                && placement.overflow != [0; 4]
            {
                let edge = overflow_edge(placement.overflow, &[]);
                return Err(ContainmentError {
                    child_id: placement.child_id,
                    parent_id: placement.parent_id,
                    message: format!(
                        "Object {} is outside parent {} on the {edge} edge at ({}, {}).",
                        placement.child_id, placement.parent_id, placement.x, placement.y
                    ),
                });
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Hash, PartialEq, Eq)]
struct PlacementKey {
    parent_id: u16,
    child_id: u16,
    context: Vec<(u16, u16)>,
}

#[derive(Clone, Debug)]
struct Placement {
    child_id: u16,
    parent_id: u16,
    child_index: usize,
    x: i32,
    y: i32,
    overflow: [i32; 4],
}

fn placements(
    pool: &ObjectPool,
    mask_size: u16,
    soft_key_size: Option<(u16, u16)>,
    version: VtVersion,
) -> HashMap<PlacementKey, Vec<Placement>> {
    let mut output = HashMap::new();
    let mut visited_parents = HashSet::new();
    for object in pool
        .objects_by_type(ag_iso_stack::object_pool::ObjectType::DataMask)
        .into_iter()
        .chain(pool.objects_by_type(ag_iso_stack::object_pool::ObjectType::AlarmMask))
    {
        traverse_parent(
            pool,
            object,
            Bounds {
                x: 0,
                y: 0,
                width: mask_size as i32,
                height: mask_size as i32,
            },
            Bounds {
                x: 0,
                y: 0,
                width: mask_size as i32,
                height: mask_size as i32,
            },
            vec![(object.id().value(), object.id().value())],
            soft_key_size,
            version,
            &mut HashSet::new(),
            &mut visited_parents,
            &mut output,
        );
    }

    // Detached bounded objects still own child lists. Treat their origins as
    // zero so their contents remain constrained without inventing a mask.
    for object in pool.objects() {
        if !matches!(object, Object::Button(_) | Object::Container(_))
            && !(soft_key_size.is_some()
                && (matches!(object, Object::Key(_)) || is_fixed_designator(object)))
        {
            continue;
        }
        if crate::operations::object_refs(object).is_none() {
            continue;
        }
        let id = object.id().value();
        if visited_parents.contains(&id) && !matches!(object, Object::Key(_)) {
            continue;
        }
        let size = bounds_for_object_with_soft_key_size(pool, object, soft_key_size);
        let outer = Bounds { x: 0, y: 0, ..size };
        traverse_parent(
            pool,
            object,
            outer,
            outer,
            vec![(id, id)],
            soft_key_size,
            version,
            &mut HashSet::new(),
            &mut visited_parents,
            &mut output,
        );
    }
    output
}

#[allow(clippy::too_many_arguments)]
fn traverse_parent(
    pool: &ObjectPool,
    parent_ref: &Object,
    parent_outer: Bounds,
    inherited_content: Bounds,
    path: Vec<(u16, u16)>,
    soft_key_size: Option<(u16, u16)>,
    version: VtVersion,
    ancestry: &mut HashSet<u16>,
    visited_parents: &mut HashSet<u16>,
    output: &mut HashMap<PlacementKey, Vec<Placement>>,
) {
    let Some(parent) = resolve_pointer(pool, parent_ref) else {
        return;
    };
    let parent_id = parent.id().value();
    if !ancestry.insert(parent_id) {
        return;
    }
    visited_parents.insert(parent_id);
    let parent_content = match parent {
        Object::Button(_) | Object::Container(_) => content_bounds(parent, parent_outer, version),
        object if is_fixed_designator(object) && soft_key_size.is_some() => parent_outer,
        Object::Key(_) => inherited_content,
        // Other wrappers inherit the viewport available at their placement.
        _ => inherited_content,
    };
    let Some((refs, owner_id)) = positioned_children(pool, parent) else {
        ancestry.remove(&parent_id);
        return;
    };
    for (child_index, reference) in refs.iter().enumerate() {
        let child_id = reference.id.value();
        let Some(raw_child) = pool.object_by_id(reference.id) else {
            continue;
        };
        let Some(child) = resolve_pointer(pool, raw_child) else {
            continue;
        };
        let child_size = bounds_for_object_with_soft_key_size(pool, child, soft_key_size);
        let base = if matches!(parent, Object::Button(_) | Object::Container(_)) {
            (parent_content.x, parent_content.y)
        } else {
            (parent_outer.x, parent_outer.y)
        };
        let child_outer = Bounds {
            x: base.0 + reference.offset.x as i32,
            y: base.1 + reference.offset.y as i32,
            width: child_size.width,
            height: child_size.height,
        };
        let mut child_path = path.clone();
        child_path.push((parent_id, child_id));
        let bounded_parent = is_bounded_parent(parent);
        let next_path = if is_bounded_parent(child) {
            Vec::new()
        } else {
            child_path
        };
        let key = PlacementKey {
            parent_id: owner_id,
            child_id,
            context: if bounded_parent && owner_id == parent_id {
                Vec::new()
            } else {
                path.clone()
            },
        };
        output.entry(key).or_default().push(Placement {
            child_id,
            parent_id: owner_id,
            child_index,
            x: reference.offset.x as i32,
            y: reference.offset.y as i32,
            overflow: edge_overflow(parent_content, child_outer),
        });

        // The child's descendants keep the same inherited viewport through
        // unbounded wrappers, while sized parents establish their own bounds.
        traverse_parent(
            pool,
            child,
            child_outer,
            parent_content,
            next_path,
            soft_key_size,
            version,
            ancestry,
            visited_parents,
            output,
        );
    }
    ancestry.remove(&parent_id);
}

fn is_bounded_parent(object: &Object) -> bool {
    matches!(
        object,
        Object::DataMask(_)
            | Object::AlarmMask(_)
            | Object::Button(_)
            | Object::Container(_)
            | Object::WorkingSet(_)
            | Object::AuxiliaryFunctionType1(_)
            | Object::AuxiliaryInputType1(_)
            | Object::AuxiliaryFunctionType2(_)
            | Object::AuxiliaryInputType2(_)
    )
}

fn is_fixed_designator(object: &Object) -> bool {
    matches!(
        object,
        Object::WorkingSet(_)
            | Object::AuxiliaryFunctionType1(_)
            | Object::AuxiliaryInputType1(_)
            | Object::AuxiliaryFunctionType2(_)
            | Object::AuxiliaryInputType2(_)
    )
}

fn uses_soft_key_dimensions(object: &Object) -> bool {
    matches!(
        object,
        Object::Key(_)
            | Object::WorkingSet(_)
            | Object::AuxiliaryFunctionType1(_)
            | Object::AuxiliaryInputType1(_)
            | Object::AuxiliaryFunctionType2(_)
            | Object::AuxiliaryInputType2(_)
            | Object::AuxiliaryControlDesignatorType2(_)
    )
}

fn edge_overflow(parent: Bounds, child: Bounds) -> [i32; 4] {
    [
        (parent.x - child.x).max(0),
        (parent.y - child.y).max(0),
        (child.x + child.width - (parent.x + parent.width)).max(0),
        (child.y + child.height - (parent.y + parent.height)).max(0),
    ]
}

/// Returns a current placement that cannot be injectively matched to a prior
/// placement with at least as much overflow on every edge. Grouping by object
/// ancestry and child ID lets reordered duplicate references retain their
/// history without allowing one prior overflow allowance to be reused.
fn unmatched_overflow(current: &[Placement], previous: &[Placement]) -> Option<usize> {
    let pending: Vec<usize> = current
        .iter()
        .enumerate()
        .filter_map(|(index, placement)| (placement.overflow != [0; 4]).then_some(index))
        .collect();
    if pending.is_empty() {
        return None;
    }

    let mut candidates: Vec<(usize, Vec<usize>)> = pending
        .into_iter()
        .map(|current_index| {
            let values = previous
                .iter()
                .enumerate()
                .filter_map(|(previous_index, placement)| {
                    placement
                        .overflow
                        .iter()
                        .zip(current[current_index].overflow)
                        .all(|(old, new)| *old >= new)
                        .then_some(previous_index)
                })
                .collect();
            (current_index, values)
        })
        .collect();
    candidates.sort_by_key(|(_, values)| values.len());

    let mut assigned = vec![None; previous.len()];
    for (current_index, _) in &candidates {
        let mut seen = vec![false; previous.len()];
        if !assign_overflow(*current_index, &candidates, &mut assigned, &mut seen) {
            return Some(*current_index);
        }
    }
    None
}

fn assign_overflow(
    current_index: usize,
    candidates: &[(usize, Vec<usize>)],
    assigned: &mut [Option<usize>],
    seen: &mut [bool],
) -> bool {
    let Some((_, choices)) = candidates
        .iter()
        .find(|(candidate_index, _)| *candidate_index == current_index)
    else {
        return false;
    };
    for previous_index in choices {
        if seen[*previous_index] {
            continue;
        }
        seen[*previous_index] = true;
        let can_take = match assigned[*previous_index] {
            Some(other_current) => assign_overflow(other_current, candidates, assigned, seen),
            None => true,
        };
        if can_take {
            assigned[*previous_index] = Some(current_index);
            return true;
        }
    }
    false
}

fn overflow_edge(overflow: [i32; 4], previous: &[Placement]) -> &'static str {
    let edge_names = ["left", "top", "right", "bottom"];
    edge_names
        .into_iter()
        .enumerate()
        .find_map(|(edge, name)| {
            (overflow[edge] > 0
                && previous
                    .iter()
                    .map(|placement| placement.overflow[edge])
                    .max()
                    .unwrap_or(0)
                    < overflow[edge])
                .then_some(name)
        })
        .or_else(|| {
            overflow
                .iter()
                .position(|value| *value > 0)
                .map(|index| edge_names[index])
        })
        .unwrap_or("unknown")
}

fn resolve_pointer<'a>(pool: &'a ObjectPool, object: &'a Object) -> Option<&'a Object> {
    let mut current = object;
    let mut seen = HashSet::new();
    loop {
        let Object::ObjectPointer(pointer) = current else {
            return Some(current);
        };
        if !seen.insert(current.id().value()) {
            return None;
        }
        current = pool.object_by_id(pointer.value.0?)?;
    }
}

fn dynamic_size(
    pool: &ObjectPool,
    object: &Object,
    soft_key_size: Option<(u16, u16)>,
    visiting: &mut HashSet<u16>,
) -> (i32, i32) {
    let id = object.id().value();
    if !visiting.insert(id) {
        return (0, 0);
    }
    let mut width = 0;
    let mut height = 0;
    if let Some((refs, _owner_id)) = positioned_children(pool, object) {
        for ObjectRef { id, offset } in refs {
            let Some(child) = pool
                .object_by_id(*id)
                .and_then(|obj| resolve_pointer(pool, obj))
            else {
                continue;
            };
            let child_size = bounds_for_object_inner(pool, child, soft_key_size, visiting);
            width = width.max(offset.x as i32 + child_size.width);
            height = height.max(offset.y as i32 + child_size.height);
        }
    }
    visiting.remove(&id);
    (width.max(0), height.max(0))
}

/// Resolve children that the preview renderer paints through a virtual
/// auxiliary designator. The designator owns no list, but its type-0 pointer
/// paints the referenced auxiliary object's positioned children at its origin.
fn positioned_children<'a>(
    pool: &'a ObjectPool,
    object: &'a Object,
) -> Option<(&'a [ObjectRef], u16)> {
    if let Object::AuxiliaryControlDesignatorType2(designator) = object {
        if designator.pointer_type == 0 {
            let target = pool.object_by_id(designator.auxiliary_object_id.0?)?;
            return match target {
                Object::AuxiliaryFunctionType2(auxiliary) => {
                    Some((&auxiliary.object_refs, target.id().value()))
                }
                Object::AuxiliaryInputType2(auxiliary) => {
                    Some((&auxiliary.object_refs, target.id().value()))
                }
                _ => None,
            };
        } else if designator.pointer_type == 2 {
            let target = pool
                .objects()
                .iter()
                .find(|candidate| matches!(candidate, Object::WorkingSet(_)))?;
            if let Object::WorkingSet(working_set) = target {
                return Some((&working_set.object_refs, target.id().value()));
            }
        }
    }
    crate::operations::object_refs(object).map(|refs| (refs, object.id().value()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ag_iso_stack::object_pool::{object_attributes::Point, ObjectId, ObjectRef, ObjectType};

    #[test]
    fn new_designator_placement_uses_soft_key_size_for_mask_containment() {
        for designator_type in [
            ObjectType::WorkingSet,
            ObjectType::AuxiliaryFunctionType1,
            ObjectType::AuxiliaryInputType1,
            ObjectType::AuxiliaryFunctionType2,
            ObjectType::AuxiliaryInputType2,
        ] {
            let mut pool = ObjectPool::default();
            for (id, object_type) in [
                (2, ObjectType::DataMask),
                (100, designator_type),
                (101, ObjectType::OutputRectangle),
            ] {
                let mut object = crate::default_object(object_type);
                object.mut_id().set_value(id).unwrap();
                pool.add(object);
            }
            if let Some(Object::DataMask(mask)) = pool.object_mut_by_id(ObjectId::new(2).unwrap()) {
                mask.object_refs.push(ObjectRef {
                    id: ObjectId::new(100).unwrap(),
                    offset: Point { x: 490, y: 0 },
                });
            }
            if let Some(object) = pool.object_mut_by_id(ObjectId::new(100).unwrap()) {
                designator_children_mut(object).push(ObjectRef {
                    id: ObjectId::new(101).unwrap(),
                    offset: Point { x: 0, y: 0 },
                });
            }

            let error = validate_new_placements_with_soft_key_size(
                &pool,
                500,
                &HashSet::from([(2, 0)]),
                Some((100, 80)),
                VtVersion::Version6,
            )
            .unwrap_err();
            assert_eq!((error.parent_id, error.child_id), (2, 100));
        }
    }

    fn designator_children_mut(object: &mut Object) -> &mut Vec<ObjectRef> {
        match object {
            Object::WorkingSet(value) => &mut value.object_refs,
            Object::AuxiliaryFunctionType1(value) => &mut value.object_refs,
            Object::AuxiliaryInputType1(value) => &mut value.object_refs,
            Object::AuxiliaryFunctionType2(value) => &mut value.object_refs,
            Object::AuxiliaryInputType2(value) => &mut value.object_refs,
            _ => unreachable!(),
        }
    }
}
