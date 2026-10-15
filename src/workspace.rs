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
}
impl Pane {
    pub const ALL: &'static [Self] = &[
        Self::Objects,
        Self::Graphics,
        Self::Components,
        Self::Canvas,
        Self::Properties,
        Self::Ancestry,
    ];
    pub fn title(self) -> &'static str {
        match self {
            Self::Objects => "Objects",
            Self::Graphics => "Graphics",
            Self::Components => "Components",
            Self::Canvas => "Canvas",
            Self::Properties => "Properties",
            Self::Ancestry => "Selection Ancestry",
        }
    }
}

#[derive(Serialize, Deserialize)]
pub struct Workspace {
    #[serde(deserialize_with = "deserialize_expanded_dock")]
    pub dock: DockState<Pane>,
    pub ancestry_expanded: bool,
}

/// Loads a dock layout with all panes expanded. egui_dock keeps collapse state
/// inside private tree fields, so normalize its serialized representation before
/// deserializing it. The rest of the saved layout, including split fractions,
/// active tabs, and window state, is left untouched.
fn deserialize_expanded_dock<'de, D>(deserializer: D) -> Result<DockState<Pane>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    // Accept the retired pane only while loading persisted layouts. Remove it
    // through DockState so empty leaves/windows and focus are reconciled normally.
    #[derive(Clone, PartialEq, Serialize, Deserialize)]
    enum StoredPane {
        Objects,
        Graphics,
        Components,
        Canvas,
        Properties,
        Ancestry,
        Proposal,
    }
    let mut dock = DockState::<StoredPane>::deserialize(deserializer)?;
    let metadata = serde_json::to_value(&dock).map_err(serde::de::Error::custom)?;
    let focused = metadata["focused_surface"].as_u64().and_then(|surface| {
        let surface = egui_dock::SurfaceIndex(usize::try_from(surface).ok()?);
        let tree = dock.get_surface(surface)?.node_tree()?;
        let node = tree.focused_leaf()?;
        let leaf = tree.iter().nth(node.0)?.get_leaf()?;
        leaf.tabs.get(leaf.active.0).cloned()
    });
    while let Some(location) = dock.find_tab(&StoredPane::Proposal) {
        dock.remove_tab(location);
    }
    if let Some(location) = focused.and_then(|pane| dock.find_tab(&pane)) {
        dock.set_active_tab(location);
        dock.set_focused_node_and_surface((location.0, location.1));
    }
    sanitize_nonfinite_dock_geometry(&mut dock);
    let mut value = serde_json::to_value(dock).map_err(serde::de::Error::custom)?;
    clear_dock_collapse_state(&mut value);
    DockState::<Pane>::deserialize(value).map_err(serde::de::Error::custom)
}

fn sanitize_nonfinite_dock_geometry<T>(dock: &mut DockState<T>) {
    for surface in dock.iter_surfaces_mut() {
        for node in surface.iter_nodes_mut() {
            if node.rect().is_some_and(|rect| !rect.is_finite()) {
                node.set_rect(egui::Rect::ZERO);
            }
            if let egui_dock::Node::Leaf(leaf) = node {
                if !leaf.viewport.is_finite() {
                    leaf.viewport = egui::Rect::ZERO;
                }
            }
        }
    }
}

fn clear_dock_collapse_state(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::Object(object) => {
            for (key, value) in object {
                match key.as_str() {
                    "collapsed" | "fully_collapsed" => *value = serde_json::Value::Bool(false),
                    "collapsed_leaf_count" => *value = serde_json::Value::from(0),
                    _ => clear_dock_collapse_state(value),
                }
            }
        }
        serde_json::Value::Array(values) => {
            for value in values {
                clear_dock_collapse_state(value);
            }
        }
        _ => {}
    }
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
    fn saved_collapsed_layout_restores_expanded_without_changing_other_state() {
        let preferences = AppPreferences {
            show_graphics: false,
            ..Default::default()
        };
        let mut workspace = Workspace::new(&preferences);
        workspace.open(Pane::Properties);
        workspace.ancestry_expanded = true;
        workspace.close(Pane::Components);
        let surface = workspace.dock.add_window(vec![Pane::Components]);
        if let Some(window) = workspace.dock.get_window_state_mut(surface) {
            window.set_position(egui::pos2(123.0, 234.0));
            window.set_size(egui::vec2(456.0, 345.0));
        }

        for surface in workspace.dock.iter_surfaces_mut() {
            for (index, node) in surface.iter_nodes_mut().enumerate() {
                // Existing panes have been shown and have finite geometry.
                let rect = egui::Rect::from_min_size(
                    egui::pos2(index as f32 * 100.0, 0.0),
                    egui::vec2(90.0, 80.0),
                );
                node.set_rect(rect);
                if let egui_dock::Node::Leaf(leaf) = node {
                    leaf.viewport = rect;
                }
            }
        }
        // Splitting in a newly opened pane can leave its initial geometry at
        // Rect::NOTHING until the next frame.
        workspace.dock.main_surface_mut().split_right(
            NodeIndex::root(),
            0.63,
            vec![Pane::Graphics],
        );
        let mut expected_dock = workspace.dock.clone();
        sanitize_nonfinite_dock_geometry(&mut expected_dock);
        for surface in workspace.dock.iter_surfaces_mut() {
            for node in surface.iter_nodes_mut() {
                match node {
                    egui_dock::Node::Leaf(_) => node.set_collapsed(true),
                    egui_dock::Node::Horizontal(_) | egui_dock::Node::Vertical(_) => {
                        node.set_collapsed(true);
                        node.set_collapsed_leaf_count(1);
                    }
                    egui_dock::Node::Empty => {}
                }
            }
        }
        let mut storage = Storage::default();
        eframe::set_value(&mut storage, STORAGE_KEY, &workspace);

        let restored: Workspace = eframe::get_value(&storage, STORAGE_KEY).unwrap();
        assert!(restored.valid());
        assert!(restored.ancestry_expanded);
        assert!(restored
            .dock
            .iter_surfaces()
            .flat_map(|surface| surface.iter_nodes())
            .all(|node| !node.is_collapsed() && node.collapsed_leaf_count() == 0));

        // This also checks that tab order and active selections, split fractions,
        // and (on native builds) floating window placement survive restoration.
        let mut expected = serde_json::to_value(expected_dock).unwrap();
        clear_dock_collapse_state(&mut expected);
        assert_eq!(serde_json::to_value(&restored.dock).unwrap(), expected);
    }

    #[test]
    fn retired_proposal_tabs_are_removed_from_saved_layouts() {
        let mut workspace = Workspace::new(&AppPreferences::default());
        workspace.close(Pane::Graphics);
        let retired = workspace.dock.add_window(vec![Pane::Graphics]);
        workspace.close(Pane::Components);
        let retained = workspace.dock.add_window(vec![Pane::Components]);
        workspace
            .dock
            .get_window_state_mut(retained)
            .unwrap()
            .set_position(egui::pos2(123.0, 234.0));
        let (surface, node, tab) = workspace.dock.find_tab(&Pane::Components).unwrap();
        workspace.dock.set_active_tab((surface, node, tab));
        workspace.dock.set_focused_node_and_surface((surface, node));
        let mut storage = Storage::default();
        eframe::set_value(&mut storage, STORAGE_KEY, &workspace);
        let saved = storage.0.get_mut(STORAGE_KEY).unwrap();
        assert!(saved.contains("Graphics"));
        *saved = saved.replace("Graphics", "Proposal");
        let restored: Workspace = eframe::get_value(&storage, STORAGE_KEY).unwrap();
        assert!(restored.valid());
        assert!(!restored.contains(Pane::Graphics));
        assert!(restored.contains(Pane::Canvas));
        assert!(restored.contains(Pane::Components));
        assert_eq!(
            restored.dock.iter_all_tabs().count(),
            workspace.dock.iter_all_tabs().count() - 1
        );
        assert_eq!(restored.dock.focused_leaf(), Some((surface, node)));
        let mut actual = serde_json::to_value(&restored.dock).unwrap();
        workspace.dock.remove_surface(retired);
        workspace.dock.set_focused_node_and_surface((surface, node));
        sanitize_nonfinite_dock_geometry(&mut workspace.dock);
        let mut expected = serde_json::to_value(&workspace.dock).unwrap();
        clear_dock_collapse_state(&mut actual);
        clear_dock_collapse_state(&mut expected);
        assert_eq!(actual, expected);
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
