//! Small command-to-draft-action parser shared by the CLI and command files.
//!
//! The public CLI accepts ordinary words and `property=value` pairs. JSON is
//! used only for the private bridge payload sent to the editor.

use serde_json::{json, Value};

fn target(value: &str) -> Value {
    if let Some(name) = value.strip_prefix("name:") {
        json!({"name": name})
    } else if let Ok(id) = if let Some(hex) = value.strip_prefix("0x") {
        u16::from_str_radix(hex, 16)
    } else {
        value.parse::<u16>()
    } {
        json!({"id": id})
    } else {
        json!({"name": value})
    }
}

fn op(name: &str, data: Value) -> Value {
    json!({"kind":"operation", "operation":{"type":name, "data":data}})
}

/// Parse one editing command (without the leading `aitd`) into bridge actions.
/// Each command is atomic at the draft layer; callers can combine returned
/// actions into one atomic batch.
pub fn parse_edit(tokens: &[String]) -> Result<Vec<Value>, String> {
    let (group, rest) = tokens.split_first().ok_or("expected an editing command")?;
    let (verb, args) = rest
        .split_first()
        .ok_or_else(|| format!("expected a {group} subcommand"))?;
    let action = match (group.as_str(), verb.as_str()) {
        ("object", "create") if args.len() == 2 && !args[1].starts_with("--") => op(
            "CreateObject",
            json!({
                "object_id": null, "object_type": args[0], "name": args[1]
            }),
        ),
        ("object", "create") if args.len() == 3 && args[1] == "--name" => op(
            "CreateObject",
            json!({"object_id":null,"object_type":args[0],"name":args[2]}),
        ),
        ("object", "rename") if args.len() >= 2 => op(
            "RenameObject",
            json!({
                "object": target(&args[0]), "name": args[1..].join(" ")
            }),
        ),
        ("object", "delete") if args.len() == 1 => {
            json!({"kind":"relationship","object":target(&args[0]),
            "operation":{"type":"DeleteObject","data":{"object_id":0}}})
        }
        ("object", "duplicate") if (1..=2).contains(&args.len()) => {
            json!({"kind":"object_duplicate","object_id":target_id(&args[0])?,"name":args.get(1)})
        }
        ("object", "field") if args.len() == 4 && args[0] == "set" => {
            let (field, value) = args[3].split_once('=').ok_or("expected field=value")?;
            json!({"kind":"object_field_set","object":target(&args[1]),"property":args[2],"field":field,"value":typed_scalar(value)})
        }
        ("object", "array") if args.len() >= 3 => {
            let object = target(&args[1]);
            let property = &args[2];
            match args[0].as_str() {
                "set" if args.len() == 5 => {
                    json!({"kind":"object_array_set","object":object,"property":property,"index":target_id(&args[3])?,"value":typed_scalar(&args[4])})
                }
                "append" if args.len() == 4 => {
                    json!({"kind":"object_array_append","object":object,"property":property,"value":typed_scalar(&args[3])})
                }
                "remove" if args.len() == 4 => {
                    json!({"kind":"object_array_remove","object":object,"property":property,"index":target_id(&args[3])?})
                }
                "clear" if args.len() == 3 => {
                    json!({"kind":"object_array_clear","object":object,"property":property})
                }
                _ => return Err("use object array set|append|remove|clear".into()),
            }
        }
        ("object", "set") if args.len() >= 2 => {
            let object = target(&args[0]);
            let mut values = serde_json::Map::new();
            for assignment in &args[1..] {
                let (property, value) = assignment
                    .split_once('=')
                    .ok_or_else(|| format!("expected property=value, got '{assignment}'"))?;
                if property.is_empty() {
                    return Err("property name cannot be empty".into());
                }
                values.insert(property.to_owned(), Value::String(value.to_owned()));
            }
            return Ok(vec![
                json!({"kind":"set_properties","object":object,"values":values}),
            ]);
        }
        ("component", "instantiate") if args.len() == 4 => {
            json!({"kind":"component","action":{"type":"InstantiateComponent","component_id":args[0],"parent":target_id(&args[1])?,"x":args[2].parse::<i32>().map_err(|_|"x must be an integer")?,"y":args[3].parse::<i32>().map_err(|_|"y must be an integer")?}})
        }
        ("component", "move") if args.len() == 3 => {
            json!({"kind":"component_move","instance_id":args[0],"x":args[1].parse::<i32>().map_err(|_|"x must be an integer")?,"y":args[2].parse::<i32>().map_err(|_|"y must be an integer")?})
        }
        ("component", "duplicate") if args.len() == 3 => {
            json!({"kind":"component_duplicate","instance_id":args[0],"x":args[1].parse::<i32>().map_err(|_|"x must be an integer")?,"y":args[2].parse::<i32>().map_err(|_|"y must be an integer")?})
        }
        ("component", "delete") if args.len() == 1 => {
            json!({"kind":"component_delete","instance_id":args[0]})
        }
        ("component", "detach") if args.len() == 1 => {
            json!({"kind":"component_detach","instance_id":args[0]})
        }
        ("component", "override") if args.len() == 3 => {
            json!({"kind":"component_override","instance_id":args[0],"parameter":args[1],"value":args[2]})
        }
        ("component", "reset") if args.len() == 2 => {
            json!({"kind":"component_reset","instance_id":args[0],"parameter":args[1]})
        }
        ("component", "capture") if args.len() >= 2 => {
            json!({"kind":"component_capture","root":target_id(&args[0])?,"name":args[1..].join(" ")})
        }
        ("component", "definition-rename") if args.len() >= 2 => {
            json!({"kind":"component_definition_rename","component_id":args[0],"name":args[1..].join(" ")})
        }
        ("component", "definition-duplicate") if args.len() == 1 => {
            json!({"kind":"component_definition_duplicate","component_id":args[0]})
        }
        ("component", "definition-delete") if args.len() == 1 => {
            json!({"kind":"component_definition_delete","component_id":args[0]})
        }
        ("component", "node-set") if args.len() == 4 => {
            json!({"kind":"component_node_set","component_id":args[0],"node_id":args[1],"property":args[2],"value_lexical":args[3]})
        }
        ("component", "node-reference") if args.len() == 4 => {
            json!({"kind":"component_node_reference","component_id":args[0],"node_id":args[1],"property":args[2],"target":args[3]})
        }
        ("component", "parameter-add") if args.len() == 6 => {
            json!({"kind":"component_parameter_add","component_id":args[0],"parameter":{"name":args[1],"description":null,"target_node":args[2],"property":args[3],"kind":args[4],"default_value_lexical":args[5]}})
        }
        ("component", "parameter-set") if args.len() == 7 => {
            json!({"kind":"component_parameter_set","component_id":args[0],"parameter":{"id":args[1],"name":args[2],"description":null,"target_node":args[3],"property":args[4],"kind":args[5],"default_value_lexical":args[6]}})
        }
        ("component", "parameter-remove") if args.len() == 2 => {
            json!({"kind":"component_parameter_remove","component_id":args[0],"parameter_id":args[1]})
        }
        ("graphic", "assign") if args.len() == 2 => {
            json!({"kind":"graphic_assign","asset_id":args[0],"object":target_id(&args[1])?})
        }
        ("graphic", "create-picture") if args.len() == 1 => {
            json!({"kind":"graphic_create_picture","asset_id":args[0]})
        }
        ("graphic", "rename") if args.len() >= 2 => {
            json!({"kind":"graphic_rename","asset_id":args[0],"name":args[1..].join(" ")})
        }
        ("graphic", "delete") if args.len() == 1 => {
            json!({"kind":"graphic_delete","asset_id":args[0]})
        }
        ("project", "id-range") if args.len() == 4 && args[0] == "set" => {
            json!({"kind":"id_range_set","object_type":args[1],"start":target_id(&args[2])?,"end":target_id(&args[3])?})
        }
        ("project", "id-range") if args.len() == 1 && args[0] == "reset" => {
            json!({"kind":"id_range_reset"})
        }
        ("project", "settings") if args.first().is_some_and(|v| v == "set") => {
            let mut values = serde_json::Map::new();
            for assignment in &args[1..] {
                let (k, v) = assignment
                    .split_once('=')
                    .ok_or_else(|| format!("expected key=value, got '{assignment}'"))?;
                values.insert(k.to_owned(), typed_scalar(v));
            }
            json!({"kind":"settings","values":values})
        }
        ("placement", verb) if ["add", "move", "remove", "order"].contains(&verb) => {
            json!({"kind":"relationship_edit","group":"placement","verb":verb,"args":args})
        }
        ("pointer", verb) if ["set", "clear"].contains(&verb) => {
            json!({"kind":"relationship_edit","group":"pointer","verb":verb,"args":args})
        }
        ("list", verb) if ["set", "insert", "remove", "clear"].contains(&verb) => {
            json!({"kind":"relationship_edit","group":"list","verb":verb,"args":args})
        }
        ("event", verb) if ["bind", "set", "unbind", "order", "clear"].contains(&verb) => {
            json!({"kind":"relationship_edit","group":"event","verb":verb,"args":args})
        }
        ("label", verb) if ["add", "set", "remove", "clear"].contains(&verb) => {
            json!({"kind":"relationship_edit","group":"label","verb":verb,"args":args})
        }
        ("reference", "set") if args.len() == 3 => op(
            "SetObjectReference",
            json!({
                "object": target(&args[0]), "property": args[1],
                "reference": if args[2] == "null" { Value::Null } else { target(&args[2]) }
            }),
        ),
        ("reference", "clear") if args.len() == 2 => op(
            "SetObjectReference",
            json!({"object":target(&args[0]),"property":args[1],"reference":null}),
        ),
        ("id", "set") if args.len() == 2 => op(
            "SetObjectIdAssignment",
            json!({
                "object_id": target_id(&args[0])?, "new_id": target_id(&args[1])?, "overridden": true
            }),
        ),
        ("id", "remap") if !args.is_empty() => {
            let mappings = args
                .iter()
                .map(|pair| {
                    let (old, new) = pair
                        .split_once(':')
                        .ok_or_else(|| format!("expected old:new ID pair, got '{pair}'"))?;
                    Ok(json!([target_id(old)?, target_id(new)?]))
                })
                .collect::<Result<Vec<_>, String>>()?;
            op(
                "RemapObjectIds",
                json!({"mappings":mappings,"target_types":[]}),
            )
        }
        ("id", "auto") if args.len() == 1 => {
            json!({"kind":"object_id_auto","object_id":target_id(&args[0])?})
        }
        ("id", "reassign") if args.is_empty() || args == ["--include-overrides"] => {
            json!({"kind":"object_id_reassign","include_overrides":!args.is_empty()})
        }
        ("object", "order") if args.len() >= 2 && args[0] == "set" => {
            let ids = args[1..]
                .iter()
                .map(|id| target_id(id))
                .collect::<Result<Vec<_>, _>>()?;
            op("ReorderObjects", json!({"object_ids":ids}))
        }
        _ => {
            return Err(format!(
                "unsupported or malformed edit: {}",
                tokens.join(" ")
            ))
        }
    };
    Ok(vec![action])
}

fn target_id(value: &str) -> Result<u16, String> {
    let number = value.strip_prefix("0x").unwrap_or(value);
    let radix = if value.starts_with("0x") { 16 } else { 10 };
    u16::from_str_radix(number, radix).map_err(|_| format!("'{value}' must be an object ID"))
}
fn typed_scalar(value: &str) -> Value {
    if let Some(s) = value.strip_prefix("string:") {
        return Value::String(s.to_owned());
    }
    if value == "true" {
        return Value::Bool(true);
    }
    if value == "false" {
        return Value::Bool(false);
    }
    if value == "null" {
        return Value::Null;
    }
    if let Ok(n) = value.parse::<i64>() {
        return json!(n);
    }
    if let Ok(n) = value.parse::<u64>() {
        return json!(n);
    }
    if let Ok(n) = value.parse::<f64>() {
        return json!(n);
    }
    Value::String(value.to_owned())
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_plain_object_edits_without_json_input() {
        let tokens = ["object", "set", "10", "width=42", "label=Ready"].map(str::to_owned);
        let actions = parse_edit(&tokens).unwrap();
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0]["kind"], "set_properties");
        assert_eq!(actions[0]["values"]["width"], "42");
        assert_eq!(actions[0]["values"]["label"], "Ready");
    }
}
