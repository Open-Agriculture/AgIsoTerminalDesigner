//! Session-only visibility and focus state for the mask preview and hierarchy.

use crate::editor_project::EditorProject;
use ag_iso_stack::object_pool::object::Object;
use ag_iso_stack::object_pool::{ObjectId, ObjectPool};
use std::collections::HashSet;

/// The masks shown in the preview and the mask whose placement tree is open.
///
/// Visibility and focus are editor view state; changing them does not modify
/// the project or its active Working Set mask.
#[derive(Clone, Debug, Default)]
pub struct MaskViewState {
    pub visible: Vec<u16>,
    pub focused: Option<u16>,
    initialized: bool,
    suppress_navigation: bool,
}

impl MaskViewState {
    /// Reconcile deleted masks and initialize from the active mask once.
    pub fn reconcile(&mut self, project: &EditorProject) {
        let pool = project.get_pool();
        let roots = mask_ids(pool);
        if !self.initialized {
            self.initialized = true;
            let active = pool
                .working_set_object()
                .map(|working_set| working_set.active_mask.value())
                .filter(|id| roots.contains(id));
            let initial = active.or_else(|| roots.first().copied());
            self.visible = initial.into_iter().collect();
            self.focused = initial;
            return;
        }

        self.visible.retain(|id| roots.contains(id));
        self.visible.sort_by_key(|id| {
            roots
                .iter()
                .position(|candidate| candidate == id)
                .unwrap_or(usize::MAX)
        });
        if self.focused.is_some_and(|id| !roots.contains(&id)) {
            self.focused = None;
        }
    }

    /// Begin a UI frame, allowing new object-selection navigation.
    pub fn begin_frame(&mut self) {
        self.suppress_navigation = false;
    }

    /// Focus a mask and make sure it is visible.
    pub fn focus(&mut self, root: u16) {
        self.focused = Some(root);
        if !self.visible.contains(&root) {
            self.visible.push(root);
        }
    }

    /// Clear hierarchy focus without changing preview visibility.
    pub fn unfocus(&mut self) {
        self.focused = None;
        self.suppress_navigation = true;
    }

    /// Change preview visibility. Hiding the focused mask clears hierarchy focus.
    pub fn set_visible(&mut self, root: u16, visible: bool) {
        if visible {
            if !self.visible.contains(&root) {
                self.visible.push(root);
            }
        } else {
            self.visible.retain(|id| *id != root);
            if self.focused == Some(root) {
                self.focused = None;
                self.suppress_navigation = true;
            }
        }
    }

    /// Focus and reveal the first mask containing `selected`, preferring the
    /// current focus. Returns the selected mask root when one contains it.
    pub fn navigate(&mut self, project: &EditorProject, selected: u16) -> Option<u16> {
        if self.suppress_navigation {
            return None;
        }
        let pool = project.get_pool();
        let roots = mask_ids(pool);
        let preferred = self
            .focused
            .filter(|root| project_contains(project, *root, selected));
        let root = preferred.or_else(|| {
            roots
                .iter()
                .copied()
                .find(|root| project_contains(project, *root, selected))
        })?;
        self.focus(root);
        Some(root)
    }
}

/// Whether an object occurs under a mask through placements, pointers, or its
/// linked soft-key slots. Used by canvas navigation to find the owning preview.
pub(crate) fn subtree_contains(pool: &ObjectPool, root: u16, selected: u16) -> bool {
    contains_object(pool, root, selected)
}

fn project_contains(project: &EditorProject, root: u16, selected: u16) -> bool {
    if contains_object(project.get_pool(), root, selected) {
        return true;
    }
    project.component_source().instances.iter().any(|instance| {
        instance.compiled_ids.values().any(|id| *id == selected)
            && match instance.parent {
                crate::components::EditorParentRef::Primitive(parent) => {
                    contains_object(project.get_pool(), root, parent)
                }
            }
    })
}

fn mask_ids(pool: &ObjectPool) -> Vec<u16> {
    pool.objects()
        .iter()
        .filter_map(|object| match object {
            Object::DataMask(_) | Object::AlarmMask(_) => Some(object.id().value()),
            _ => None,
        })
        .collect()
}

fn mask_soft_key_id(pool: &ObjectPool, root: u16) -> Option<u16> {
    let object = pool.object_by_id(ObjectId::new(root).ok()?)?;
    match object {
        Object::DataMask(mask) => mask.soft_key_mask.0.map(|id| id.value()),
        Object::AlarmMask(mask) => mask.soft_key_mask.0.map(|id| id.value()),
        _ => None,
    }
}

fn contains_object(pool: &ObjectPool, root: u16, selected: u16) -> bool {
    fn visit(pool: &ObjectPool, current: u16, selected: u16, path: &mut HashSet<u16>) -> bool {
        if current == selected {
            return true;
        }
        if !path.insert(current) {
            return false;
        }
        let Some(object) = ObjectId::new(current)
            .ok()
            .and_then(|id| pool.object_by_id(id))
        else {
            path.remove(&current);
            return false;
        };
        if let Object::ObjectPointer(pointer) = object {
            if pointer
                .value
                .0
                .is_some_and(|target| visit(pool, target.value(), selected, path))
            {
                path.remove(&current);
                return true;
            }
        }
        if crate::operations::object_refs(object).is_some_and(|refs| {
            refs.iter()
                .any(|reference| visit(pool, reference.id.value(), selected, path))
        }) {
            path.remove(&current);
            return true;
        }
        path.remove(&current);
        false
    }

    if visit(pool, root, selected, &mut HashSet::new()) {
        return true;
    }
    let Some(soft_key_id) = mask_soft_key_id(pool, root) else {
        return false;
    };
    if soft_key_id == selected {
        return true;
    }
    let Some(Object::SoftKeyMask(soft_keys)) = ObjectId::new(soft_key_id)
        .ok()
        .and_then(|id| pool.object_by_id(id))
    else {
        return false;
    };
    soft_keys
        .objects
        .iter()
        .any(|key| visit(pool, key.value(), selected, &mut HashSet::new()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ag_iso_stack::object_pool::ObjectType;

    #[test]
    fn focus_navigation_and_empty_visibility_are_independent() {
        let mut project = EditorProject::new_pool();
        let first = project
            .create_object(ObjectType::DataMask, "First".into())
            .unwrap()
            .value();
        let second = project
            .create_object(ObjectType::AlarmMask, "Second".into())
            .unwrap()
            .value();
        let mut state = MaskViewState::default();
        state.reconcile(&project);
        assert_eq!(state.focused, Some(first));
        assert_eq!(state.visible, vec![first]);

        state.set_visible(first, false);
        assert_eq!(state.focused, None);
        assert!(state.visible.is_empty());
        state.reconcile(&project);
        assert!(state.visible.is_empty());
        assert_eq!(state.focused, None);

        state.focus(second);
        state.unfocus();
        assert_eq!(state.focused, None);
        assert_eq!(state.visible, vec![second]);
        state.begin_frame();
        assert_eq!(state.navigate(&project, second), Some(second));
        assert_eq!(state.focused, Some(second));
    }
}
