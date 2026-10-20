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
