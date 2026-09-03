//! Translate draft-object changes into the narrowest available operations.

use super::operation::{
    macro_refs, object_labels, object_list, object_refs, set_macro_refs, set_object_labels,
    set_object_list, set_object_refs,
};
use super::Operation;
use ag_iso_stack::object_pool::object::Object;

/// Build operations that reproduce `new` from `old` without replacing the
/// complete object. An error means a future object-model field has not yet been
/// assigned an explicit operation and the edit must not be committed.
pub(crate) fn diff_object(old: &Object, new: &Object) -> Result<Vec<Operation>, String> {
    if old.id() != new.id() || old.object_type() != new.object_type() {
        return Err("object identity or type changed during field diff".to_owned());
    }

    let mut candidate = old.clone();
    let mut operations = Vec::new();

    match (object_refs(old), object_refs(new)) {
        (Some(old_children), Some(new_children)) if old_children != new_children => {
            let new_children = new_children.to_vec();
            if !set_object_refs(&mut candidate, new_children.clone()) {
                return Err("positioned-child representation mismatch".to_owned());
            }
            operations.push(Operation::SetChildren {
                parent_id: old.id().value(),
                children: new_children,
            });
        }
        (Some(_), Some(_)) | (None, None) => {}
        _ => return Err("positioned-child support mismatch".to_owned()),
    }

    match (macro_refs(old), macro_refs(new)) {
        (Some(old_refs), Some(new_refs)) if old_refs != new_refs => {
            let new_refs = new_refs.to_vec();
            if !set_macro_refs(&mut candidate, new_refs.clone()) {
                return Err("macro-reference representation mismatch".to_owned());
            }
            operations.push(Operation::SetMacroReferences {
                object_id: old.id().value(),
                macro_refs: new_refs,
            });
        }
        (Some(_), Some(_)) | (None, None) => {}
        _ => return Err("macro-reference support mismatch".to_owned()),
    }

    match (object_list(old), object_list(new)) {
        (Some(old_objects), Some(new_objects)) if old_objects != new_objects => {
            if !set_object_list(&mut candidate, new_objects.clone()) {
                return Err("object-list representation mismatch".to_owned());
            }
            operations.push(Operation::SetObjectList {
                object_id: old.id().value(),
                objects: new_objects,
            });
        }
        (Some(_), Some(_)) | (None, None) => {}
        _ => return Err("object-list support mismatch".to_owned()),
    }

    match (object_labels(old), object_labels(new)) {
        (Some(old_labels), Some(new_labels)) if old_labels != new_labels => {
            let new_labels = new_labels.to_vec();
            if !set_object_labels(&mut candidate, new_labels.clone()) {
                return Err("object-label representation mismatch".to_owned());
            }
            operations.push(Operation::SetObjectLabels {
                object_id: old.id().value(),
                labels: new_labels,
            });
        }
        (Some(_), Some(_)) | (None, None) => {}
        _ => return Err("object-label support mismatch".to_owned()),
    }

    let property_changes = crate::object_properties::changed_properties(&candidate, new)
        .ok_or_else(|| format!("unmapped {:?} field change", old.object_type()))?;
    operations.extend(property_changes.into_iter().map(|(property, value)| {
        Operation::SetProperty {
            object_id: old.id().value(),
            property,
            value,
        }
    }));

    Ok(operations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::default_object;
    use ag_iso_stack::object_pool::{
        object_attributes::{Event, MacroRef, ObjectRef, Point},
        ObjectId, ObjectType,
    };
    use std::collections::HashSet;

    #[test]
    fn combines_children_macros_and_properties_without_replacing_object() {
        let mut old = default_object(ObjectType::Button);
        old.mut_id().set_value(10).unwrap();
        let mut new = old.clone();
        if let Object::Button(button) = &mut new {
            button.width = 100;
            button.object_refs.push(ObjectRef {
                id: ObjectId::new(11).unwrap(),
                offset: Point { x: 2, y: 3 },
            });
            button.macro_refs.push(MacroRef {
                macro_id: 12,
                event_id: Event::OnActivate,
            });
        }

        let operations = diff_object(&old, &new).unwrap();

        assert_eq!(operations.len(), 3);
        assert!(matches!(operations[0], Operation::SetChildren { .. }));
        assert!(matches!(
            operations[1],
            Operation::SetMacroReferences { .. }
        ));
        assert!(matches!(
            &operations[2],
            Operation::SetProperty { property, .. } if property == "width"
        ));
    }

    #[test]
    fn translates_unpositioned_object_lists() {
        let old = default_object(ObjectType::InputList);
        let mut new = old.clone();
        if let Object::InputList(list) = &mut new {
            list.list_items.push(ObjectId::new(2).unwrap().into());
        }

        let operations = diff_object(&old, &new).unwrap();
        assert!(matches!(
            operations.as_slice(),
            [Operation::SetObjectList { .. }]
        ));
    }

    #[test]
    fn translates_object_labels() {
        use ag_iso_stack::object_pool::object_attributes::ObjectLabel;

        let old = default_object(ObjectType::ObjectLabelReferenceList);
        let mut new = old.clone();
        if let Object::ObjectLabelReferenceList(list) = &mut new {
            list.object_labels.push(ObjectLabel {
                id: ObjectId::new(2).unwrap(),
                string_variable_reference: ObjectId::new(3).unwrap().into(),
                font_type: 0,
                graphic_representation: ObjectId::new(4).unwrap().into(),
            });
        }

        let operations = diff_object(&old, &new).unwrap();
        assert!(matches!(
            operations.as_slice(),
            [Operation::SetObjectLabels { .. }]
        ));
    }

    #[test]
    fn every_serialized_object_field_has_an_explicit_operation_route() {
        let all_object_types = (0..=u8::MAX).filter_map(|value| ObjectType::try_from(value).ok());

        for object_type in all_object_types {
            let object = default_object(object_type);
            let serialized = serde_json::to_value(&object).unwrap();
            let actual: HashSet<_> = serialized["properties"]
                .as_object()
                .unwrap()
                .keys()
                .map(String::as_str)
                .collect();
            let mut mapped: HashSet<_> =
                crate::object_properties::get_property_descriptors(object_type)
                    .into_iter()
                    .map(|descriptor| descriptor.name)
                    .collect();
            mapped.insert("id");
            if object_refs(&object).is_some() {
                mapped.insert("object_refs");
            }
            if macro_refs(&object).is_some() {
                mapped.insert("macro_refs");
            }
            if object_list(&object).is_some() {
                mapped.insert(match object_type {
                    ObjectType::InputList | ObjectType::OutputList => "list_items",
                    _ => "objects",
                });
            }
            if object_labels(&object).is_some() {
                mapped.insert("object_labels");
            }

            assert_eq!(
                actual, mapped,
                "missing operation route for {object_type:?}"
            );
        }
    }
}
