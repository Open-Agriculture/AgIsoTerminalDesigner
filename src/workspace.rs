//! Application-owned dock layout; tabs carry identity, never project data.
use crate::designer_settings::AppPreferences;
use eframe::egui;
use egui_dock::{DockState, NodeIndex};
use serde::{Deserialize, Serialize};

pub const STORAGE_KEY: &str = "designer_workspace_v1";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Pane {
    Objects,
    Graphics,
    Components,
    Canvas,
    Properties,
    Ancestry,
    #[cfg(not(target_arch = "wasm32"))]
    Proposal,
}
impl Pane {
    pub const ALL: &'static [Self] = &[
        Self::Objects,
        Self::Graphics,
        Self::Components,
        Self::Canvas,
        Self::Properties,
        Self::Ancestry,
        #[cfg(not(target_arch = "wasm32"))]
        Self::Proposal,
    ];
    pub fn title(self) -> &'static str {
        match self {
            Self::Objects => "Objects",
            Self::Graphics => "Graphics",
            Self::Components => "Components",
            Self::Canvas => "Canvas",
            Self::Properties => "Properties",
            Self::Ancestry => "Selection Ancestry",
            #[cfg(not(target_arch = "wasm32"))]
            Self::Proposal => "Proposal",
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct Workspace {
    pub dock: DockState<Pane>,
    pub ancestry_expanded: bool,
}
impl Workspace {
    pub fn new(preferences: &AppPreferences) -> Self {
        let mut dock = DockState::new(vec![Pane::Canvas]);
        let left = [
            (preferences.show_objects, Pane::Objects),
            (preferences.show_graphics, Pane::Graphics),
            (preferences.show_components, Pane::Components),
        ]
        .into_iter()
        .filter_map(|(visible, pane)| visible.then_some(pane))
        .collect::<Vec<_>>();
        let mut center = NodeIndex::root();
        if !left.is_empty() {
            center = dock.main_surface_mut().split_left(center, 0.22, left)[0];
        }
        if preferences.show_properties {
            center = dock
                .main_surface_mut()
                .split_right(center, 0.72, vec![Pane::Properties])[0];
        }
        if preferences.show_ancestry {
            dock.main_surface_mut()
                .split_below(center, 0.88, vec![Pane::Ancestry]);
        }
        #[cfg(not(target_arch = "wasm32"))]
        if preferences.show_proposal {
            let surface = dock.add_window(vec![Pane::Proposal]);
            if let Some(window) = dock.get_window_state_mut(surface) {
                window.set_position(egui::pos2(70.0, 80.0));
                window.set_size(egui::vec2(360.0, 420.0));
            }
        }
        Self {
            dock,
            ancestry_expanded: false,
        }
    }
    pub fn contains(&self, pane: Pane) -> bool {
        self.dock.find_tab(&pane).is_some()
    }
    pub fn open(&mut self, pane: Pane) {
        if let Some((surface, node, tab)) = self.dock.find_tab(&pane) {
            self.dock.set_active_tab((surface, node, tab));
            self.dock.set_focused_node_and_surface((surface, node));
        } else {
            self.dock.push_to_focused_leaf(pane);
        }
    }
    pub fn close(&mut self, pane: Pane) {
        if let Some(location) = self.dock.find_tab(&pane) {
            self.dock.remove_tab(location);
        }
    }
    pub fn valid(&self) -> bool {
        use egui_dock::{Node, Surface};
        let surfaces = self.dock.iter_surfaces().collect::<Vec<_>>();
        if !matches!(surfaces.first(), Some(Surface::Main(_))) {
            return false;
        }
        let mut seen = std::collections::HashSet::new();
        for surface in surfaces {
            let Some(tree) = surface.node_tree() else {
                continue;
            };
            if let Some(focused) = tree.focused_leaf() {
                if !tree.iter().nth(focused.0).is_some_and(Node::is_leaf) {
                    return false;
                }
            }
            let mut pane_count = 0;
            for (index, node) in tree.iter().enumerate() {
                match node {
                    Node::Leaf(leaf) => {
                        if leaf.tabs.is_empty() || leaf.active.0 >= leaf.tabs.len() {
                            return false;
                        }
                        for pane in &leaf.tabs {
                            if !seen.insert(*pane) {
                                return false;
                            }
                            pane_count += 1;
                        }
                    }
                    Node::Horizontal(split) | Node::Vertical(split) => {
                        if !split.fraction.is_finite() || !(0.0..=1.0).contains(&split.fraction) {
                            return false;
                        }
                        if ![index * 2 + 1, index * 2 + 2].into_iter().all(|child| {
                            tree.iter().nth(child).is_some_and(|node| !node.is_empty())
                        }) {
                            return false;
                        }
                    }
                    Node::Empty => {}
                }
            }
            if matches!(surface, Surface::Window(..)) && pane_count == 0 {
                return false;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[derive(Default)]
    struct Storage(std::collections::HashMap<String, String>);
    impl eframe::Storage for Storage {
        fn get_string(&self, key: &str) -> Option<String> {
            self.0.get(key).cloned()
        }
        fn set_string(&mut self, key: &str, value: String) {
            self.0.insert(key.into(), value);
        }
        fn flush(&mut self) {}
    }
    #[test]
    fn saved_layout_preserves_visibility_and_active_pane() {
        let preferences = AppPreferences {
            show_graphics: false,
            ..Default::default()
        };
        let mut workspace = Workspace::new(&preferences);
        assert!(!workspace.contains(Pane::Graphics));
        workspace.close(Pane::Components);
        workspace.open(Pane::Graphics);
        workspace.ancestry_expanded = true;
        let mut storage = Storage::default();
        eframe::set_value(&mut storage, STORAGE_KEY, &workspace);
        let restored: Workspace = eframe::get_value(&storage, STORAGE_KEY).unwrap();
        assert!(restored.valid());
        assert!(restored.ancestry_expanded);
        assert!(restored.contains(Pane::Graphics));
        assert!(!restored.contains(Pane::Components));
        let (surface, node, tab) = restored.dock.find_tab(&Pane::Graphics).unwrap();
        assert_eq!(restored.dock[surface][node].get_leaf().unwrap().active, tab);
    }
    #[test]
    fn reopening_does_not_duplicate_panes_and_invalid_layout_is_rejected() {
        let mut workspace = Workspace::new(&AppPreferences::default());
        for _ in 0..3 {
            workspace.open(Pane::Objects);
        }
        workspace.close(Pane::Canvas);
        workspace.open(Pane::Canvas);
        assert!(workspace.valid());
        assert_eq!(
            workspace
                .dock
                .iter_all_tabs()
                .filter(|(_, pane)| **pane == Pane::Objects)
                .count(),
            1
        );
        let (surface, node, _) = workspace.dock.find_tab(&Pane::Canvas).unwrap();
        workspace.dock[surface][node].get_leaf_mut().unwrap().active = egui_dock::TabIndex(999);
        assert!(!workspace.valid());
    }
}
