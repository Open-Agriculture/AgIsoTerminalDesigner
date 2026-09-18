//! AI-only compilation. The canonical transaction and executor remain primitive.
use super::AiDesignState;
use crate::{
    effective_colour as colours,
    object_properties::{get_property_descriptors, PropertySemantic},
};
use ag_iso_stack::object_pool::{ObjectPool, ObjectType};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum AiColourValue {
    Index(u8),
    Name { name: String },
    Rgb { rgb: String },
}
impl AiColourValue {
    pub fn resolve(&self, pool: &ObjectPool) -> Result<colours::ResolvedColour, Value> {
        match self {
            Self::Index(i)=>Ok(colours::resolve_index(pool,*i)),
            Self::Name{name}=>colours::resolve_name(pool,name).ok_or_else(||json!({"code":"AI_COLOUR_UNKNOWN","requested":name,"available_names":["black","white","dark_blue","blue","cyan","red","green","gray","light_gray","yellow","teal","purple","maroon","olive","lime","magenta"]})),
            Self::Rgb{rgb}=>parse_rgb(rgb).map(|rgb|colours::nearest(pool,rgb)).ok_or_else(||json!({"code":"AI_COLOUR_INVALID","requested":rgb,"format":"#RRGGBB"})),
        }
    }
}
fn parse_rgb(s: &str) -> Option<[u8; 3]> {
    if s.len() != 7 || !s.starts_with('#') || !s.is_ascii() {
        return None;
    }
    Some([
        u8::from_str_radix(&s[1..3], 16).ok()?,
        u8::from_str_radix(&s[3..5], 16).ok()?,
        u8::from_str_radix(&s[5..7], 16).ok()?,
    ])
}
pub fn resolve(pool: &ObjectPool, v: &Value) -> Result<colours::ResolvedColour, Value> {
    serde_json::from_value::<AiColourValue>(v.clone())
        .map_err(|e| json!({"code":"AI_COLOUR_INVALID","message":e.to_string()}))?
        .resolve(pool)
}
pub fn colour_schema() -> Value {
    json!({"anyOf":[{"type":"integer","minimum":0,"maximum":255},{"type":"object","properties":{"name":{"type":"string"}},"required":["name"],"additionalProperties":false},{"type":"object","properties":{"rgb":{"type":"string","pattern":"^#[0-9A-Fa-f]{6}$"}},"required":["rgb"],"additionalProperties":false}]})
}
pub fn theme_schema() -> Value {
    let mut props = json!({});
    for role in [
        "background",
        "primary_text",
        "secondary_text",
        "accent",
        "danger",
    ] {
        props[role] = json!({"anyOf":[colour_schema(),{"type":"null"}]});
    }
    json!({"type":"object","properties":props,"required":["background","primary_text","secondary_text","accent","danger"],"additionalProperties":false})
}
pub fn resolve_theme(pool: &ObjectPool, theme: &Value) -> Result<Value, Value> {
    if theme.is_null() {
        return Ok(json!({}));
    }
    let Some(roles) = theme.as_object() else {
        return Err(json!({"code":"AI_THEME_INVALID"}));
    };
    let mut result = json!({});
    for (role, value) in roles {
        if ![
            "background",
            "primary_text",
            "secondary_text",
            "accent",
            "danger",
        ]
        .contains(&role.as_str())
        {
            return Err(json!({"code":"AI_THEME_INVALID","role":role}));
        }
        if !value.is_null() {
            result[role] = json!(resolve(pool, value)?);
        }
    }
    Ok(result)
}
pub fn enrich(pool: &ObjectPool, ty: ObjectType, name: &str, value: Value) -> Value {
    if get_property_descriptors(ty)
        .iter()
        .any(|d| d.name == name && matches!(d.semantic, PropertySemantic::Colour))
    {
        if let Some(i) = value.as_u64().and_then(|i| u8::try_from(i).ok()) {
            return json!(colours::resolve_index(pool, i));
        }
    }
    value
}
/// Normalize only descriptor-declared Colour properties, including newly created objects.
pub fn compile(state: &AiDesignState, arguments: Value) -> Result<(Value, Vec<Value>), Value> {
    let first = compile_with_palette(state, &state.pool, arguments.clone())?;
    // Explicit palette edits are unusual. Prepare a disposable candidate through
    // the normal executor, then resolve against its final effective palette.
    // No canonical state or history is changed by either preparation.
    let palette_edit = arguments["operations"]
        .as_array()
        .into_iter()
        .flatten()
        .any(|op| {
            let d = &op["data"];
            matches!(
                d["object_type"].as_str(),
                Some("ColourPalette" | "ColourMap" | "WorkingSetSpecialControls")
            ) || matches!(
                d["property"].as_str(),
                Some("colours" | "colour_map" | "id_of_colour_map" | "id_of_colour_palette")
            ) || d["object_id"]
                .as_u64()
                .and_then(|n| ag_iso_stack::object_pool::ObjectId::new(n as u16).ok())
                .and_then(|id| state.pool.object_by_id(id))
                .is_some_and(|o| {
                    matches!(
                        o.object_type(),
                        ObjectType::ColourPalette
                            | ObjectType::ColourMap
                            | ObjectType::WorkingSetSpecialControls
                    )
                })
        });
    if palette_edit {
        let mut envelope = first.0.clone();
        envelope["schema_version"] = json!(crate::operations::CURRENT_OPERATION_SCHEMA_VERSION);
        let tx = serde_json::from_value(envelope).map_err(
            |e| json!({"code":"AI_PALETTE_TRANSACTION_INVALID","message":e.to_string()}),
        )?;
        let candidate = super::propose_transaction(state, tx)
            .map_err(|e| json!({"code":"AI_PALETTE_TRANSACTION_INVALID","diagnostics":e}))?;
        return compile_with_palette(state, &candidate.prepared.pool, arguments);
    }
    Ok(first)
}
fn compile_with_palette(
    state: &AiDesignState,
    palette: &ObjectPool,
    mut arguments: Value,
) -> Result<(Value, Vec<Value>), Value> {
    let mut ids = BTreeMap::new();
    let mut names = BTreeMap::new();
    for object in state.pool.objects() {
        ids.insert(object.id().value(), object.object_type());
        if let Some(name) = state
            .object_info
            .get(&object.id())
            .and_then(|i| i.name.as_ref())
        {
            names.insert(name.clone(), object.object_type());
        }
    }
    let mut output = Vec::new();
    let mut metadata = Vec::new();
    let ops = arguments["operations"]
        .as_array()
        .ok_or_else(|| json!({"code":"AI_OPERATIONS_INVALID"}))?;
    if ops.len() > 1000 {
        return Err(json!({"code":"AI_OPERATIONS_TOO_LARGE"}));
    }
    for op in ops {
        let expanded = if op["type"] == "CreateIcon" {
            super::icon_recipes::compile(palette, &op["data"])?
        } else {
            vec![op.clone()]
        };
        for mut op in expanded {
            let kind = op["type"].as_str().unwrap_or("").to_string();
            let d = &mut op["data"];
            if kind == "CreateObject" {
                if let Some(ty) = ObjectType::values()
                    .into_iter()
                    .find(|t| json!(t) == d["object_type"])
                {
                    if let Some(id) = d["object_id"].as_u64().and_then(|n| u16::try_from(n).ok()) {
                        ids.insert(id, ty);
                    }
                    if let Some(name) = d["name"].as_str() {
                        names.insert(name.into(), ty);
                    }
                }
            }
            if kind == "ChangeObjectId" {
                if let (Some(old), Some(new)) = (d["old_id"].as_u64(), d["new_id"].as_u64()) {
                    if let Some(ty) = ids.remove(&(old as u16)) {
                        ids.insert(new as u16, ty);
                    }
                }
            }
            if kind == "RenameObject" {
                let ty = d["object"]["id"]
                    .as_u64()
                    .and_then(|id| ids.get(&(id as u16)))
                    .or_else(|| d["object"]["name"].as_str().and_then(|n| names.get(n)))
                    .copied();
                if let (Some(ty), Some(name)) = (ty, d["name"].as_str()) {
                    names.insert(name.into(), ty);
                }
            }
            if kind == "SetProperty" {
                let ty = d["object"]["id"]
                    .as_u64()
                    .and_then(|i| ids.get(&(i as u16)))
                    .or_else(|| d["object"]["name"].as_str().and_then(|n| names.get(n)))
                    .copied();
                if let Some(ty) = ty {
                    let name = d["property"].as_str().unwrap_or("");
                    if get_property_descriptors(ty)
                        .iter()
                        .any(|p| p.name == name && matches!(p.semantic, PropertySemantic::Colour))
                    {
                        let requested = d["value"].clone();
                        let resolved = resolve(palette, &requested)?;
                        let delta = serde_json::from_value::<AiColourValue>(requested.clone())
                            .ok()
                            .and_then(|c| match c {
                                AiColourValue::Name { name } => colours::named_rgb(&name),
                                AiColourValue::Rgb { rgb } => parse_rgb(&rgb),
                                _ => None,
                            })
                            .map(|rgb| {
                                colours::distance(
                                    rgb,
                                    colours::colour(palette, resolved.index).as_rgb(),
                                )
                            });
                        metadata.push(json!({"object":d["object"],"property":name,"requested":requested,"role":d["theme_role"],"resolved":resolved,"distance_squared_rgb":delta,"source":colours::source(palette)}));
                        d["value"] = json!(resolved.index);
                    }
                }
            }
            if kind == "SetProperty" {
                if let Some(fields) = d.as_object_mut() {
                    fields.remove("theme_role");
                }
            }
            output.push(op);
        }
    }
    arguments["operations"] = json!(output);
    Ok((arguments, metadata))
}
/// Only a mask background has an unambiguous implicit role. Accent variations are valid.
pub fn theme_mismatches(proposal: &super::AiProposal, theme: &Value) -> Vec<Value> {
    use crate::operations::{ObjectTarget, Operation};
    let mut out = Vec::new();
    for op in &proposal.transaction.operations {
        if let Operation::SetProperty {
            object, property, ..
        } = op
        {
            let id = match object {
                ObjectTarget::Id { id } => Some(*id),
                ObjectTarget::Name { name } => proposal
                    .prepared
                    .object_info
                    .iter()
                    .find(|(_, i)| i.name.as_ref() == Some(name))
                    .map(|(id, _)| id.value()),
            };
            let obj = id
                .and_then(|id| ag_iso_stack::object_pool::ObjectId::new(id).ok())
                .and_then(|id| proposal.prepared.pool.object_by_id(id));
            if property == "background_colour"
                && obj.is_some_and(|o| {
                    matches!(
                        o.object_type(),
                        ObjectType::DataMask | ObjectType::AlarmMask
                    )
                })
                && !theme["background"].is_null()
            {
                if let Some(index) = obj
                    .and_then(|o| crate::object_properties::get_object_property(o, property).ok())
                    .and_then(|v| v.as_u64())
                    .and_then(|v| u8::try_from(v).ok())
                {
                    let actual = colours::resolve_index(&proposal.prepared.pool, index);
                    if json!(actual.rgb) != theme["background"]["rgb"] {
                        out.push(json!({"code":"STYLE_INTENT_MISMATCH","role":"background","object":object,"expected":theme["background"],"actual":actual}));
                    }
                }
            }
        }
    }
    out
}

/// Explicit role annotations compile away and never enter history.
pub fn explicit_theme_mismatches(
    pool: &ObjectPool,
    resolutions: &Value,
    theme: &Value,
) -> Vec<Value> {
    resolutions.as_array().into_iter().flatten().filter_map(|r|{
        let role=r["role"].as_str()?;let expected=theme.get(role)?;if expected.is_null(){return None}
        let actual=colours::resolve_index(pool,r["resolved"]["index"].as_u64()? as u8);
        (json!(actual.rgb)!=expected["rgb"]).then(||json!({"code":"STYLE_INTENT_MISMATCH","role":role,"object":r["object"],"property":r["property"],"expected":expected,"actual":actual}))
    }).collect()
}
