//! Resolved colour diagnostics for design inspection.
use super::{AiDesignState, AiProposal};
use crate::{
    effective_colour as colours,
    object_properties::{get_object_property, get_property_descriptors, PropertySemantic},
};
use ag_iso_stack::object_pool::{ObjectId, ObjectPool};
use serde_json::{json, Value};
#[derive(Debug, Clone, Copy)]
pub struct ContrastThresholds {
    pub text: f64,
    pub button: f64,
    pub status: f64,
}
impl Default for ContrastThresholds {
    fn default() -> Self {
        Self {
            text: 4.5,
            button: 4.5,
            status: 4.5,
        }
    }
}
pub fn colour_summary(pool: &ObjectPool) -> Value {
    let entries:Vec<_>=pool.objects().iter().filter_map(|o|{let mut properties=json!({});for d in get_property_descriptors(o.object_type()){if matches!(d.semantic,PropertySemantic::Colour){if let Ok(v)=get_object_property(o,d.name){properties[d.name]=super::visual_intent::enrich(pool,o.object_type(),d.name,v);}}} (!properties.as_object().unwrap().is_empty()).then(||json!({"id":o.id().value(),"type":format!("{:?}",o.object_type()),"colours":properties}))}).take(65).collect();
    json!({"objects":entries.iter().take(64).collect::<Vec<_>>(),"truncated":entries.len()>64})
}
pub fn shared_style_impact(state: &AiDesignState, p: &AiProposal) -> Vec<Value> {
    let mut result = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for op in &p.transaction.operations {
        if let crate::operations::Operation::SetProperty {
            object, property, ..
        } = op
        {
            let id = match object {
                crate::operations::ObjectTarget::Id { id } => ObjectId::new(*id).ok(),
                crate::operations::ObjectTarget::Name { name } => state
                    .object_info
                    .iter()
                    .find(|(_, i)| i.name.as_ref() == Some(name))
                    .map(|(id, _)| *id),
            };
            if let Some(id) = id {
                if let Some(o) = state.pool.object_by_id(id) {
                    if !matches!(
                        o.object_type(),
                        ag_iso_stack::object_pool::ObjectType::FontAttributes
                            | ag_iso_stack::object_pool::ObjectType::FillAttributes
                            | ag_iso_stack::object_pool::ObjectType::LineAttributes
                    ) {
                        continue;
                    }
                    if get_property_descriptors(o.object_type()).iter().any(|d| {
                        d.name == property && matches!(d.semantic, PropertySemantic::Colour)
                    }) && seen.insert(id.value())
                    {
                        let users = state.pool.parent_objects(id);
                        if users.len() > 1 {
                            let mut visited = std::collections::BTreeSet::new();
                            let mut frontier = vec![id];
                            let mut masks = std::collections::BTreeSet::new();
                            while let Some(next) = frontier.pop() {
                                if !visited.insert(next.value()) {
                                    continue;
                                }
                                for parent in state.pool.parent_objects(next) {
                                    if format!("{:?}", parent.object_type()).ends_with("Mask") {
                                        masks.insert(parent.id().value());
                                    }
                                    frontier.push(parent.id());
                                }
                            }
                            result.push(json!({"code":"SHARED_STYLE_SIDE_EFFECT","severity":"Warning","style_id":id.value(),"property":property,"affected_objects_count":users.len(),"affected_masks_count":masks.len(),"affected_objects":users.iter().take(16).map(|o|o.id().value()).collect::<Vec<_>>(),"guidance":"For a local edit, create dedicated attributes and redirect the target reference. Modify this shared style only for global intent."}));
                        }
                    }
                }
            }
        }
    }
    result
}
pub fn contrast_diagnostics(
    pool: &ObjectPool,
    info: &std::collections::HashMap<ObjectId, crate::ObjectInfo>,
    layout: &[super::snapshot::LayoutInstance],
    threshold: ContrastThresholds,
) -> Vec<super::DesignDiagnostic> {
    let mut result = Vec::new();
    for item in layout {
        let Some(object) = ObjectId::new(item.object.id)
            .ok()
            .and_then(|id| pool.object_by_id(id))
        else {
            continue;
        };
        let reference = get_object_property(object, "font_attributes")
            .or_else(|_| get_object_property(object, "foreground_colour"));
        let Some(font) = reference
            .ok()
            .and_then(|v| v.as_u64())
            .and_then(|id| ObjectId::new(id as u16).ok())
            .and_then(|id| pool.object_by_id(id))
        else {
            continue;
        };
        let Some(fg) = get_object_property(font, "font_colour")
            .ok()
            .and_then(|v| v.as_u64())
        else {
            continue;
        };
        let mut background = None;
        let mut button = false;
        for id in std::iter::once(item.object.id).chain(
            item.path
                .iter()
                .rev()
                .copied()
                .filter(|id| *id != item.object.id),
        ) {
            let Some(o) = ObjectId::new(id).ok().and_then(|id| pool.object_by_id(id)) else {
                continue;
            };
            button |= matches!(
                o.object_type(),
                ag_iso_stack::object_pool::ObjectType::Button
                    | ag_iso_stack::object_pool::ObjectType::Key
            );
            let options = get_object_property(o, "options").unwrap_or(Value::Null);
            if options["transparent"] == true || options["transparent_background"] == true {
                continue;
            }
            if let Some(bg) = get_object_property(o, "background_colour")
                .ok()
                .and_then(|v| v.as_u64())
            {
                background = Some((bg as u8, id));
                break;
            }
        }
        // Button context includes ancestors even when the label paints its own background.
        button |= item.path.iter().any(|id| {
            ObjectId::new(*id)
                .ok()
                .and_then(|id| pool.object_by_id(id))
                .is_some_and(|o| {
                    matches!(
                        o.object_type(),
                        ag_iso_stack::object_pool::ObjectType::Button
                            | ag_iso_stack::object_pool::ObjectType::Key
                    )
                })
        });
        let Some((bg, bg_id)) = background else {
            continue;
        };
        let status = item.object.name.to_ascii_lowercase().contains("status");
        let limit = if button {
            threshold.button
        } else if status {
            threshold.status
        } else {
            threshold.text
        };
        let ratio = colours::contrast(
            colours::colour(pool, fg as u8).as_rgb(),
            colours::colour(pool, bg).as_rgb(),
        );
        if ratio < limit {
            let details = json!({"foreground":colours::resolve_index(pool,fg as u8),"background":colours::resolve_index(pool,bg),"contrast_ratio":ratio,"threshold":limit,"object_id":item.object.id,"font_attributes":font.id().value(),"background_object":bg_id,"path":item.path});
            result.push(super::DesignDiagnostic {
                severity: super::DesignDiagnosticSeverity::Warning,
                code: if button {
                    "LOW_BUTTON_CONTRAST"
                } else if status {
                    "LOW_STATUS_CONTRAST"
                } else {
                    "LOW_TEXT_CONTRAST"
                }
                .into(),
                message: format!(
                    "Text contrast {ratio:.2}:1 is below {limit:.2}:1 (readability heuristic)."
                ),
                context: Some(details),
                objects: super::snapshot::summary(pool, info, object.id())
                    .into_iter()
                    .collect(),
                region: None,
            });
        }
    }
    result
}
