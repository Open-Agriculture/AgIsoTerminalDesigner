//! Terminal preview geometry and paging, following ISO 11783-6 §4.5.3 and B.5.
use crate::{EditorProject, SoftKeyArrangement};
use ag_iso_stack::object_pool::{object::Object, ObjectId, ObjectPool};
use egui::Rect;
use std::collections::HashSet;

pub(super) const PATH_PREFIX: usize = usize::MAX;

#[derive(Clone, Debug)]
pub(super) enum SlotContent {
    Key(u16),
    Empty,
    Invalid,
    Previous,
    Next,
}
#[derive(Clone, Debug)]
pub(super) struct Slot {
    pub rect: Rect,
    pub index: usize,
    pub content: SlotContent,
}
pub(super) struct SoftKeys {
    pub mask_id: Option<u16>,
    pub background: u8,
    pub rect: Rect,
    pub slots: Vec<Slot>,
    pub page: usize,
    pub pages: usize,
    pub navigation: u8,
    pub error: Option<String>,
}

fn resolve(pool: &ObjectPool, id: u16) -> SlotContent {
    let mut current = id;
    let mut seen = HashSet::new();
    while seen.insert(current) {
        match ObjectId::new(current)
            .ok()
            .and_then(|id| pool.object_by_id(id))
        {
            Some(Object::Key(_)) => return SlotContent::Key(current),
            Some(Object::ObjectPointer(pointer)) => match pointer.value.0 {
                Some(id) => current = id.value(),
                None => return SlotContent::Empty,
            },
            _ => return SlotContent::Invalid,
        }
    }
    SlotContent::Invalid
}

pub(super) fn build(
    project: &EditorProject,
    pool: &ObjectPool,
    root: u16,
    page: usize,
) -> SoftKeys {
    let reference = match ObjectId::new(root)
        .ok()
        .and_then(|id| pool.object_by_id(id))
    {
        Some(Object::DataMask(mask)) => mask.soft_key_mask.0,
        Some(Object::AlarmMask(mask)) => mask.soft_key_mask.0,
        _ => None,
    };
    let mut result = SoftKeys {
        mask_id: reference.map(|id| id.value()),
        background: 0,
        rect: Rect::NOTHING,
        slots: vec![],
        page: 0,
        pages: 1,
        navigation: 0,
        error: None,
    };
    let Some(id) = reference else {
        return result;
    };
    let Some(Object::SoftKeyMask(mask)) = pool.object_by_id(id) else {
        result.error = Some(format!(
            "Missing or invalid softkey mask {}",
            project.format_object_id(id.value())
        ));
        return result;
    };
    result.background = mask.background_colour;
    let layout = project.soft_key_layout();
    let physical = usize::from(layout.physical_key_count);
    let per_line = usize::from(layout.keys_per_line);
    let mut contents: Vec<_> = mask
        .objects
        .iter()
        .map(|id| resolve(pool, id.value()))
        .collect();
    while matches!(contents.last(), Some(SlotContent::Empty)) {
        contents.pop();
    }
    result.navigation = if contents.len() > physical {
        layout.navigation_key_count
    } else {
        0
    };
    let capacity = physical - usize::from(result.navigation);
    result.pages = contents.len().div_ceil(capacity).max(1);
    result.page = page.min(result.pages - 1);
    let lines = physical.div_ceil(per_line);
    let (width, height) = project.soft_key_size();
    let (w, h) = (f32::from(width), f32::from(height));
    // Gaps are outside the designator content, including its separator stroke.
    let gap = 4.0;
    let mask_gap = 12.0;
    let (area_origin, area_size) = match layout.arrangement {
        SoftKeyArrangement::Right => (
            egui::pos2(f32::from(project.mask_size) + mask_gap, 0.0),
            egui::vec2(
                lines as f32 * (w + gap) - gap,
                per_line as f32 * (h + gap) - gap,
            ),
        ),
        SoftKeyArrangement::Bottom => (
            egui::pos2(0.0, f32::from(project.mask_size) + mask_gap),
            egui::vec2(
                per_line as f32 * (w + gap) - gap,
                lines as f32 * (h + gap) - gap,
            ),
        ),
    };
    result.rect = Rect::from_min_size(area_origin, area_size);
    for physical_index in 0..physical {
        let line = physical_index / per_line;
        let within = physical_index % per_line;
        let offset = match layout.arrangement {
            SoftKeyArrangement::Right => egui::vec2(
                (lines - 1 - line) as f32 * (w + gap),
                within as f32 * (h + gap),
            ),
            SoftKeyArrangement::Bottom => {
                egui::vec2(within as f32 * (w + gap), line as f32 * (h + gap))
            }
        };
        let index = result.page * capacity + physical_index;
        let content = if physical_index < capacity {
            contents.get(index).cloned().unwrap_or(SlotContent::Empty)
        } else if result.navigation == 2 && physical_index == capacity {
            SlotContent::Previous
        } else {
            SlotContent::Next
        };
        result.slots.push(Slot {
            rect: Rect::from_min_size(area_origin + offset, egui::vec2(w, h)),
            index,
            content,
        });
    }
    result
}

pub(super) fn area(path: &[usize]) -> &[usize] {
    if path.first() == Some(&PATH_PREFIX) {
        &path[..path.len().min(2)]
    } else {
        &[]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{default_object, SoftKeyLayout};
    use ag_iso_stack::object_pool::{NullableObjectId, ObjectType};

    fn project() -> EditorProject {
        let mut pool = ObjectPool::default();
        for (kind, id) in [
            (ObjectType::DataMask, 1),
            (ObjectType::SoftKeyMask, 2),
            (ObjectType::Key, 3),
            (ObjectType::ObjectPointer, 4),
        ] {
            let mut object = default_object(kind);
            object.mut_id().set_value(id).unwrap();
            match &mut object {
                Object::DataMask(mask) => {
                    mask.soft_key_mask = NullableObjectId(Some(ObjectId::new(2).unwrap()))
                }
                Object::SoftKeyMask(mask) => {
                    mask.objects = vec![ObjectId::new(3).unwrap(); 13];
                    mask.objects[1] = ObjectId::new(4).unwrap();
                    mask.objects.push(ObjectId::new(4).unwrap());
                }
                _ => {}
            }
            pool.add(object);
        }
        EditorProject::from(pool)
    }

    #[test]
    fn paging_preserves_internal_nulls_and_ignores_trailing_nulls() {
        let project = project();
        let keys = build(&project, project.get_pool(), 1, 0);
        assert_eq!(keys.pages, 2);
        assert!(matches!(keys.slots[1].content, SlotContent::Empty));
        assert!(matches!(keys.slots[10].content, SlotContent::Previous));
        assert!(matches!(keys.slots[11].content, SlotContent::Next));
        assert!(keys.slots[0].rect.left() > keys.slots[6].rect.left());
        assert_eq!(keys.slots[0].rect.top(), keys.slots[6].rect.top());
        let last = build(&project, project.get_pool(), 1, 99);
        assert_eq!(last.page, 1);
        assert!(matches!(last.slots[2].content, SlotContent::Key(3)));
        assert!(matches!(last.slots[3].content, SlotContent::Empty));
    }

    #[test]
    fn bottom_layout_fits_all_keys_without_reserving_navigation() {
        let mut project = project();
        project
            .set_project_configuration(
                project.mask_size,
                (80, 40),
                project.vt_version(),
                SoftKeyLayout {
                    arrangement: SoftKeyArrangement::Bottom,
                    physical_key_count: 13,
                    keys_per_line: 6,
                    navigation_key_count: 2,
                },
            )
            .unwrap();
        let keys = build(&project, project.get_pool(), 1, 0);
        assert_eq!(keys.pages, 1);
        assert_eq!(keys.navigation, 0);
        assert!(keys.slots[0].rect.right() < keys.slots[1].rect.left());
        assert!(keys.slots[6].rect.top() > keys.slots[0].rect.bottom());
        assert_eq!(keys.slots[0].rect.size(), egui::vec2(80.0, 40.0));
        assert!(keys.slots[0].rect.top() > f32::from(project.mask_size));
    }
}
