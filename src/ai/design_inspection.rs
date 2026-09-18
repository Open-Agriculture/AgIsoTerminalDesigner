//! Focused display-tree inspection using the existing path-aware layout iterator.
use super::{AiDesignState, AiToolError};
use crate::object_properties::get_object_property;
use ag_iso_stack::object_pool::ObjectId;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;
pub const DEFAULT_REGION_RESULTS: usize = 12;
pub const MAX_REGION_RESULTS: usize = 30;
pub const MAX_REGION_DEPTH: usize = 6;
const MAX_VISITED: usize = 4096;
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Region {
    Top,
    Bottom,
    Left,
    Right,
    Center,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    #[default]
    All,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct InspectDesignRegionRequest {
    pub root_id: u16,
    pub region: Region,
    pub types: Vec<String>,
    pub purpose: String,
    pub max_results: usize,
}
impl Default for InspectDesignRegionRequest {
    fn default() -> Self {
        Self {
            root_id: 0,
            region: Region::All,
            types: vec![],
            purpose: "style_match".into(),
            max_results: DEFAULT_REGION_RESULTS,
        }
    }
}
pub fn inspect_design_region(
    state: &AiDesignState,
    request: InspectDesignRegionRequest,
) -> Result<Value, AiToolError> {
    let root =
        ObjectId::new(request.root_id).map_err(|_| AiToolError::ObjectNotFound(request.root_id))?;
    let object = state
        .pool
        .object_by_id(root)
        .ok_or(AiToolError::ObjectNotFound(request.root_id))?;
    let dimension = |name| {
        get_object_property(object, name)
            .ok()
            .and_then(|v| v.as_f64())
            .unwrap_or(state.mask_size as f64)
    };
    let (width, height) = (dimension("width"), dimension("height"));
    let (x0, y0, x1, y1) = match request.region {
        Region::Top => (0., 0., width, height / 2.),
        Region::Bottom => (0., height / 2., width, height),
        Region::Left => (0., 0., width / 2., height),
        Region::Right => (width / 2., 0., width, height),
        Region::Center => (width / 4., height / 4., width * 0.75, height * 0.75),
        Region::TopLeft => (0., 0., width / 2., height / 2.),
        Region::TopRight => (width / 2., 0., width, height / 2.),
        Region::BottomLeft => (0., height / 2., width / 2., height),
        Region::BottomRight => (width / 2., height / 2., width, height),
        Region::All => (0., 0., width, height),
    };
    let mut origins = BTreeMap::from([(vec![request.root_id], (0i32, 0i32))]);
    let mut candidates = Vec::new();
    let mut visited = 0;
    let mut depth_limited = false;
    for p in super::snapshot::context_layout(
        &state.pool,
        &state.object_info,
        Some(root),
        MAX_REGION_DEPTH,
    )
    .take(MAX_VISITED + 1)
    {
        visited += 1;
        if visited > MAX_VISITED {
            break;
        }
        let parent_path = &p.path[..p.path.len() - 1];
        let (px, py) = origins.get(parent_path).copied().unwrap_or_default();
        let (ax, ay) = (px + p.x, py + p.y);
        origins.insert(p.path.clone(), (ax, ay));
        let object = state
            .pool
            .object_by_id(ObjectId::new(p.object.id).unwrap())
            .unwrap();
        depth_limited |=
            p.depth == MAX_REGION_DEPTH && !super::snapshot::display_children(object).is_empty();
        if !request.types.is_empty() && !request.types.contains(&p.object.object_type) {
            continue;
        }
        let (cx, cy) = (
            ax as f64 + p.width.unwrap_or(0) as f64 / 2.,
            ay as f64 + p.height.unwrap_or(0) as f64 / 2.,
        );
        if !matches!(request.region, Region::All) && !(cx >= x0 && cx < x1 && cy >= y0 && cy < y1) {
            continue;
        }
        let mut style = json!({});
        for name in [
            "background_colour",
            "border_colour",
            "transparency_colour",
            "justification",
            "options",
            "horizontal_justification",
            "vertical_justification",
        ] {
            if let Ok(v) = get_object_property(object, name) {
                style[name] = super::visual_intent::enrich(&state.pool, object.object_type(), name, v);
            }
        }
        let mut refs = json!({});
        for name in [
            "font_attributes",
            "fill_attributes",
            "line_attributes",
            "variable_reference",
        ] {
            if let Ok(v) = get_object_property(object, name) {
                if let Ok(id) =
                    serde_json::from_value::<ag_iso_stack::object_pool::NullableObjectId>(v)
                {
                    refs[name] = json!(id.0.map(|id| id.value()));
                    if name != "variable_reference" {
                        if let Some(attribute) = id.0.and_then(|id| state.pool.object_by_id(id)) {
                            let mut resolved = json!({});
                            for d in crate::object_properties::get_property_descriptors(
                                attribute.object_type(),
                            ) {
                                if let Ok(v) = get_object_property(attribute, d.name) {
                                    if v.to_string().len() <= 256 {
                                        resolved[d.name] = super::visual_intent::enrich(&state.pool, attribute.object_type(), d.name, v);
                                    }
                                }
                            }
                            style[name] = resolved;
                        }
                    }
                }
            }
        }
        let distance = (cx - (x0 + x1) / 2.).powi(2) + (cy - (y0 + y1) / 2.).powi(2);
        candidates.push((distance,p.path.clone(),json!({"id":p.object.id,"name":p.object.name.chars().take(128).collect::<String>(),"type":p.object.object_type,"path":p.path,"parent":p.parent.id,
            "local":{"x":p.x,"y":p.y,"width":p.width,"height":p.height},
            "absolute":{"x":ax,"y":ay,"width":p.width,"height":p.height},"style":style,"refs":refs})));
    }
    candidates.sort_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)));
    let count = candidates.len();
    let limit = request.max_results.clamp(1, MAX_REGION_RESULTS);
    let objects: Vec<_> = candidates
        .into_iter()
        .take(limit)
        .map(|(_, _, v)| v)
        .collect();
    Ok(
        json!({"root_id":request.root_id,"coordinate_origin":"requested_root","region":request.region,"objects":objects,
        "ranking":"distance_to_region_center_then_path","matched":count,"truncated":count>limit || depth_limited || visited>MAX_VISITED,
        "depth_limited":depth_limited,"scan_limited":visited>MAX_VISITED,"max_depth":MAX_REGION_DEPTH}),
    )
}
