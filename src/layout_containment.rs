//! Shared geometry checks for positioned object references.
//!
//! Bounds are in pool coordinates and describe the nominal, unclipped extent
//! used by the canvas. Placement validation deliberately visits every mask so
//! shared objects cannot be resized out of bounds in an inactive mask.

use crate::object_rendering::BUTTON_BORDER_INSET;
use ag_iso_stack::object_pool::{object::Object, object_attributes::ObjectRef, ObjectPool};
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
pub(crate) fn bounds_for_object(pool: &ObjectPool, object: &Object) -> Bounds {
    bounds_for_object_inner(pool, object, &mut HashSet::new())
}

fn bounds_for_object_inner(
    pool: &ObjectPool,
    object: &Object,
    visiting: &mut HashSet<u16>,
) -> Bounds {
    let Some(object) = resolve_pointer(pool, object) else {
        return Bounds::default();
    };
    let (width, height) = object
        .as_sized_object()
        .map(|sized| (sized.width() as i32, sized.height() as i32))
        .unwrap_or_else(|| dynamic_size(pool, object, visiting));
    Bounds {
        x: 0,
        y: 0,
        width,
        height,
    }
}

/// Returns the content area of an object within its nominal outer rectangle.
/// Buttons render their children inside the four-pixel face inset.
pub(crate) fn content_bounds(object: &Object, outer: Bounds) -> Bounds {
    if matches!(object, Object::Button(_)) {
        let inset = BUTTON_BORDER_INSET as i32;
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
pub(crate) fn validate_change(
    before: &ObjectPool,
    after: &ObjectPool,
    before_mask: u16,
    after_mask: u16,
) -> Result<(), ContainmentError> {
    validate_change_with_id_changes(before, after, before_mask, after_mask, &[])
}

/// Validates a change while preserving placement identity across object ID
/// renames. Renames are applied to baseline placement identities in order.
pub(crate) fn validate_change_with_id_changes(
    before: &ObjectPool,
    after: &ObjectPool,
    before_mask: u16,
    after_mask: u16,
    id_changes: &[(u16, u16)],
) -> Result<(), ContainmentError> {
    let before_placements = remap_placement_ids(placements(before, before_mask), id_changes);
    let after_placements = placements(after, after_mask);
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

fn remap_id(mut id: u16, id_changes: &[(u16, u16)]) -> u16 {
    for (old_id, new_id) in id_changes {
        if id == *old_id {
            id = *new_id;
        }
    }
    id
}

/// Strictly validates references introduced by the current edit, even when a
/// prior occurrence of the same child had grandfathered overflow. The caller
/// supplies IDs and list indices from the operation that added those refs.
pub(crate) fn validate_new_placements(
    after: &ObjectPool,
    mask_size: u16,
    new_refs: &HashSet<(u16, usize)>,
) -> Result<(), ContainmentError> {
    for placements in placements(after, mask_size).into_values() {
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

fn placements(pool: &ObjectPool, mask_size: u16) -> HashMap<PlacementKey, Vec<Placement>> {
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
            &mut HashSet::new(),
            &mut visited_parents,
            &mut output,
        );
    }

    // Detached bounded objects still own child lists. Treat their origins as
    // zero so their contents remain constrained without inventing a mask.
    for object in pool.objects() {
        if !matches!(object, Object::Button(_) | Object::Container(_)) {
            continue;
        }
        if crate::operations::object_refs(object).is_none() {
            continue;
        }
        let id = object.id().value();
        if visited_parents.contains(&id) {
            continue;
        }
        let size = bounds_for_object(pool, object);
        let outer = Bounds { x: 0, y: 0, ..size };
        traverse_parent(
            pool,
            object,
            outer,
            outer,
            vec![(id, id)],
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
        Object::Button(_) | Object::Container(_) => content_bounds(parent, parent_outer),
        // Key and auxiliary designator children inherit the viewport available
        // at their placement, matching the canvas renderer's clip geometry.
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
        let child_size = bounds_for_object(pool, child);
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
        Object::DataMask(_) | Object::AlarmMask(_) | Object::Button(_) | Object::Container(_)
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

fn dynamic_size(pool: &ObjectPool, object: &Object, visiting: &mut HashSet<u16>) -> (i32, i32) {
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
            let child_size = bounds_for_object_inner(pool, child, visiting);
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
    use crate::default_object;
    use ag_iso_stack::object_pool::{object_attributes::Point, ObjectId, ObjectType};

    fn object(ty: ObjectType, id: u16) -> Object {
        let mut object = default_object(ty);
        object.mut_id().set_value(id).unwrap();
        object
    }

    fn reference(id: u16, x: i16, y: i16) -> ObjectRef {
        ObjectRef {
            id: ObjectId::new(id).unwrap(),
            offset: Point { x, y },
        }
    }

    #[test]
    fn accepts_contained_children_and_rejects_a_new_overflow() {
        let mut mask = object(ObjectType::DataMask, 1);
        let mut child = object(ObjectType::OutputRectangle, 2);
        if let Object::OutputRectangle(rect) = &mut child {
            rect.width = 10;
            rect.height = 8;
        }
        if let Object::DataMask(mask) = &mut mask {
            mask.object_refs.push(reference(2, 20, 30));
        }
        let mut pool = ObjectPool::default();
        pool.add(mask);
        pool.add(child);
        assert!(validate_change(&pool, &pool, 100, 100).is_ok());

        let mut changed = pool.clone();
        if let Some(Object::DataMask(mask)) = changed.object_mut_by_id(ObjectId::new(1).unwrap()) {
            mask.object_refs[0].offset.x = 95;
        }
        let error = validate_change(&pool, &changed, 100, 100).unwrap_err();
        assert_eq!(error.code(), OBJECT_OUTSIDE_PARENT);
        assert_eq!((error.parent_id, error.child_id), (1, 2));
    }

    #[test]
    fn permits_existing_overflow_to_shrink_but_not_grow() {
        let mut mask = object(ObjectType::DataMask, 1);
        let mut child = object(ObjectType::OutputRectangle, 2);
        if let Object::OutputRectangle(rect) = &mut child {
            rect.width = 20;
            rect.height = 20;
        }
        if let Object::DataMask(mask) = &mut mask {
            mask.object_refs.push(reference(2, 95, 10));
        }
        let mut before = ObjectPool::default();
        before.add(mask);
        before.add(child);
        let mut repaired = before.clone();
        if let Some(Object::DataMask(mask)) = repaired.object_mut_by_id(ObjectId::new(1).unwrap()) {
            mask.object_refs[0].offset.x = 90;
        }
        assert!(validate_change(&before, &repaired, 100, 100).is_ok());
        if let Some(Object::DataMask(mask)) = repaired.object_mut_by_id(ObjectId::new(1).unwrap()) {
            mask.object_refs[0].offset.x = 96;
        }
        assert!(validate_change(&before, &repaired, 100, 100).is_err());
    }

    #[test]
    fn repeated_references_match_across_reorder_and_remove_without_reusing_allowance() {
        let mut mask = object(ObjectType::DataMask, 1);
        let mut child = object(ObjectType::OutputRectangle, 2);
        if let Object::OutputRectangle(rect) = &mut child {
            rect.width = 20;
            rect.height = 10;
        }
        if let Object::DataMask(mask) = &mut mask {
            mask.object_refs
                .extend([reference(2, 95, 10), reference(2, 90, 30)]);
        }
        let mut before = ObjectPool::default();
        before.add(mask);
        before.add(child);

        let mut reordered = before.clone();
        if let Some(Object::DataMask(mask)) = reordered.object_mut_by_id(ObjectId::new(1).unwrap())
        {
            mask.object_refs.swap(0, 1);
        }
        assert!(validate_change(&before, &reordered, 100, 100).is_ok());

        let mut removed = before.clone();
        if let Some(Object::DataMask(mask)) = removed.object_mut_by_id(ObjectId::new(1).unwrap()) {
            mask.object_refs.remove(0);
        }
        assert!(validate_change(&before, &removed, 100, 100).is_ok());

        let mut duplicated = before.clone();
        if let Some(Object::DataMask(mask)) = duplicated.object_mut_by_id(ObjectId::new(1).unwrap())
        {
            mask.object_refs.push(reference(2, 95, 50));
        }
        assert!(validate_change(&before, &duplicated, 100, 100).is_err());
    }

    #[test]
    fn button_inset_and_picture_aspect_are_used_for_geometry() {
        let mut button = object(ObjectType::Button, 2);
        if let Object::Button(button) = &mut button {
            button.width = 20;
            button.height = 18;
        }
        let outer = Bounds {
            x: 0,
            y: 0,
            width: 20,
            height: 18,
        };
        assert_eq!(
            content_bounds(&button, outer),
            Bounds {
                x: 4,
                y: 4,
                width: 12,
                height: 10
            }
        );

        let mut picture = object(ObjectType::PictureGraphic, 3);
        if let Object::PictureGraphic(picture) = &mut picture {
            picture.width = 20;
            picture.actual_width = 4;
            picture.actual_height = 3;
        }
        assert_eq!(
            bounds_for_object(&ObjectPool::default(), &picture).height,
            15
        );
    }

    #[test]
    fn pointer_cycles_do_not_recurse_forever() {
        let mut a = object(ObjectType::ObjectPointer, 1);
        let mut b = object(ObjectType::ObjectPointer, 2);
        if let Object::ObjectPointer(pointer) = &mut a {
            pointer.value.0 = Some(ObjectId::new(2).unwrap());
        }
        if let Object::ObjectPointer(pointer) = &mut b {
            pointer.value.0 = Some(ObjectId::new(1).unwrap());
        }
        let mut pool = ObjectPool::default();
        pool.add(a.clone());
        pool.add(b);
        assert_eq!(bounds_for_object(&pool, &a), Bounds::default());
    }

    #[test]
    fn type_zero_auxiliary_designator_uses_referenced_child_geometry() {
        let mut mask = object(ObjectType::DataMask, 1);
        let mut designator = object(ObjectType::AuxiliaryControlDesignatorType2, 2);
        let mut auxiliary = object(ObjectType::AuxiliaryFunctionType2, 3);
        let mut child = object(ObjectType::OutputRectangle, 4);
        if let Object::AuxiliaryControlDesignatorType2(designator) = &mut designator {
            designator.pointer_type = 0;
            designator.auxiliary_object_id.0 = Some(ObjectId::new(3).unwrap());
        }
        if let Object::AuxiliaryFunctionType2(auxiliary) = &mut auxiliary {
            auxiliary.object_refs.push(reference(4, -5, 0));
        }
        if let Object::OutputRectangle(rectangle) = &mut child {
            rectangle.width = 10;
            rectangle.height = 10;
        }
        if let Object::DataMask(mask) = &mut mask {
            mask.object_refs.push(reference(2, 1, 20));
        }
        let mut pool = ObjectPool::default();
        pool.add(mask);
        pool.add(designator.clone());
        pool.add(auxiliary);
        pool.add(child);

        assert_eq!(bounds_for_object(&pool, &designator).width, 5);
        let error = validate_change(&ObjectPool::default(), &pool, 100, 100).unwrap_err();
        assert_eq!((error.parent_id, error.child_id), (3, 4));
        assert!(error.message.contains("left edge"));
    }

    #[test]
    fn type_two_auxiliary_designator_uses_working_set_children() {
        let mut mask = object(ObjectType::DataMask, 1);
        let mut designator = object(ObjectType::AuxiliaryControlDesignatorType2, 2);
        let mut working_set = object(ObjectType::WorkingSet, 3);
        let mut child = object(ObjectType::OutputRectangle, 4);
        if let Object::AuxiliaryControlDesignatorType2(designator) = &mut designator {
            designator.pointer_type = 2;
        }
        if let Object::WorkingSet(working_set) = &mut working_set {
            working_set.object_refs.push(reference(4, 0, -5));
        }
        if let Object::OutputRectangle(rectangle) = &mut child {
            rectangle.width = 10;
            rectangle.height = 10;
        }
        if let Object::DataMask(mask) = &mut mask {
            mask.object_refs.push(reference(2, 20, 1));
        }
        let mut pool = ObjectPool::default();
        pool.add(mask);
        pool.add(designator.clone());
        pool.add(working_set);
        pool.add(child);

        assert_eq!(bounds_for_object(&pool, &designator).height, 5);
        let error = validate_change(&ObjectPool::default(), &pool, 100, 100).unwrap_err();
        assert_eq!((error.parent_id, error.child_id), (3, 4));
        assert!(error.message.contains("top edge"));
    }

    #[test]
    fn bounded_parent_overflow_survives_detach_and_reparent() {
        let mut mask = object(ObjectType::DataMask, 1);
        let other_mask = object(ObjectType::DataMask, 5);
        let mut container = object(ObjectType::Container, 2);
        let mut child = object(ObjectType::OutputRectangle, 3);
        if let Object::DataMask(mask) = &mut mask {
            mask.object_refs.push(reference(2, 50, 50));
        }
        if let Object::Container(container) = &mut container {
            container.width = 20;
            container.height = 20;
            container.object_refs.push(reference(3, 5, 5));
        }
        if let Object::OutputRectangle(rectangle) = &mut child {
            rectangle.width = 30;
            rectangle.height = 10;
        }
        let mut before = ObjectPool::default();
        before.add(mask);
        before.add(other_mask.clone());
        before.add(container);
        before.add(child);

        let mut detached = before.clone();
        if let Some(Object::DataMask(mask)) = detached.object_mut_by_id(ObjectId::new(1).unwrap()) {
            mask.object_refs.clear();
        }
        assert!(validate_change(&before, &detached, 100, 100).is_ok());

        if let Some(Object::DataMask(mask)) = detached.object_mut_by_id(ObjectId::new(5).unwrap()) {
            mask.object_refs.push(reference(2, 30, 40));
        }
        assert!(validate_change(&before, &detached, 100, 100).is_ok());
    }

    #[test]
    fn explicitly_new_reference_cannot_inherit_an_overflow_allowance() {
        let mut mask = object(ObjectType::DataMask, 1);
        let mut child = object(ObjectType::OutputRectangle, 2);
        if let Object::OutputRectangle(rectangle) = &mut child {
            rectangle.width = 20;
            rectangle.height = 10;
        }
        if let Object::DataMask(mask) = &mut mask {
            mask.object_refs
                .extend([reference(2, 95, 10), reference(2, 20, 30)]);
        }
        let mut before = ObjectPool::default();
        before.add(mask);
        before.add(child);

        let mut after = before.clone();
        if let Some(Object::DataMask(mask)) = after.object_mut_by_id(ObjectId::new(1).unwrap()) {
            mask.object_refs[0].offset.x = 20;
            mask.object_refs.push(reference(2, 95, 50));
        }
        assert!(validate_change(&before, &after, 100, 100).is_ok());
        let new_refs = HashSet::from([(1, 2)]);
        assert!(validate_new_placements(&after, 100, &new_refs).is_err());
    }

    #[test]
    fn object_id_changes_preserve_existing_overflow_identity() {
        let mut mask = object(ObjectType::DataMask, 1);
        let mut child = object(ObjectType::OutputRectangle, 2);
        if let Object::OutputRectangle(rectangle) = &mut child {
            rectangle.width = 20;
            rectangle.height = 10;
        }
        if let Object::DataMask(mask) = &mut mask {
            mask.object_refs.push(reference(2, 95, 10));
        }
        let mut before = ObjectPool::default();
        before.add(mask);
        before.add(child);

        let mut after = before.clone();
        if let Some(child) = after.object_mut_by_id(ObjectId::new(2).unwrap()) {
            child.mut_id().set_value(12).unwrap();
        }
        if let Some(Object::DataMask(mask)) = after.object_mut_by_id(ObjectId::new(1).unwrap()) {
            mask.object_refs[0].id = ObjectId::new(12).unwrap();
        }
        assert!(validate_change_with_id_changes(&before, &after, 100, 100, &[(2, 12)]).is_ok());

        if let Some(Object::DataMask(mask)) = after.object_mut_by_id(ObjectId::new(1).unwrap()) {
            mask.object_refs[0].offset.x = 96;
        }
        assert!(validate_change_with_id_changes(&before, &after, 100, 100, &[(2, 12)]).is_err());
    }
}
