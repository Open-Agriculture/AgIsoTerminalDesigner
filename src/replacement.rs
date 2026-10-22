//! Project-wide redirection of references while retaining both object definitions.

use crate::{
    canvas::edits::PlacementTarget, components::*, EditorProject, Operation, OperationTransaction,
};
use ag_iso_stack::object_pool::vt_version::VtVersion;
use std::collections::HashMap;

impl EditorProject {
    /// Redirect every local inbound reference from `from` to `to` as one
    /// validated, undoable project edit. Both object definitions remain.
    pub fn replace_object_references(&mut self, from: u16, to: u16) -> Result<(), String> {
        if from == to {
            return Err("Choose a different object".into());
        }
        self.get_pool()
            .object_by_id(object_id(to)?)
            .ok_or_else(|| format!("Replacement object {to} no longer exists"))?;
        let version = self.vt_version;
        let from_name = self
            .get_pool()
            .object_by_id(object_id(from)?)
            .map(|object| self.get_object_info(object).get_name(object).to_owned())
            .ok_or_else(|| format!("Object {from} no longer exists"))?;
        let baseline_pool = self
            .materialize()
            .map(|materialized| materialized.pool)
            .unwrap_or_else(|_| self.get_pool().clone());
        let baseline = crate::pool_validation::validate_complete_pool_state_for_version(
            &baseline_pool,
            version,
        );
        let mut candidate = self.clone();
        candidate
            .edit_source("Replace object references", |pool, source, _, info| {
                let before_pool = pool.clone();
                crate::operations::operation::redirect_pool_references(pool, from, to, version)
                    .map_err(|error| {
                        ComponentError::new("REPLACE_REFERENCES", error.to_string())
                    })?;
                validate_rewritten_references(&before_pool, pool, from, to, version)?;
                redirect_component_source(source, pool, from, to, &from_name, version)?;
                for (owner, object_info) in info.iter_mut() {
                    if object_info
                        .allowed_references
                        .contains(&object_id(from).unwrap())
                    {
                        for reference in &mut object_info.allowed_references {
                            if *reference == object_id(from).unwrap() {
                                *reference = object_id(to).unwrap();
                            }
                        }
                        let mut seen = std::collections::HashSet::new();
                        object_info
                            .allowed_references
                            .retain(|reference| seen.insert(*reference));
                        crate::pool_validation::would_create_circular_reference(
                            &before_pool,
                            *owner,
                            object_id(to).unwrap(),
                        )
                        .map_err(|error| {
                            ComponentError::new("REPLACE_ALLOWED_REFERENCE", error.message)
                        })?;
                    }
                }
                Ok(())
            })
            .map_err(|error| error.to_string())?;
        let materialized = candidate
            .materialize_shared()
            .map_err(|error| error.to_string())?;
        let diagnostics = crate::pool_validation::validate_complete_pool_state_for_version(
            &materialized.pool,
            version,
        );
        if let Some(error) =
            crate::editor_project::new_or_worsened_project_errors(&baseline, &diagnostics)
                .into_iter()
                .next()
        {
            return Err(error.message);
        }
        *self = candidate;
        Ok(())
    }
}

fn validate_rewritten_references(
    before: &ag_iso_stack::object_pool::ObjectPool,
    after: &ag_iso_stack::object_pool::ObjectPool,
    from: u16,
    to: u16,
    version: VtVersion,
) -> ComponentResult<()> {
    let target = object_id(to).map_err(|error| ComponentError::new("REPLACE_REFERENCES", error))?;
    for original in before.objects() {
        let owner = original.id();
        {
            for descriptor in
                crate::object_properties::get_property_descriptors(original.object_type())
            {
                if !matches!(
                    descriptor.semantic,
                    crate::object_properties::PropertySemantic::ObjectReference { .. }
                ) {
                    continue;
                }
                let Ok(value) =
                    crate::object_properties::get_property(before, owner, descriptor.name)
                else {
                    continue;
                };
                if value.as_u64() != Some(u64::from(from)) {
                    continue;
                }
                let value = crate::object_properties::get_property(after, owner, descriptor.name)
                    .map_err(|error| {
                    ComponentError::new("REPLACE_REFERENCES", error.to_string())
                })?;
                crate::object_properties::validate_property(after, owner, descriptor.name, &value)
                    .map_err(|error| {
                        ComponentError::new("REPLACE_REFERENCE_TYPE", error.to_string())
                    })?;
                crate::pool_validation::would_create_circular_reference(before, owner, target)
                    .map_err(|error| {
                        ComponentError::new("REPLACE_REFERENCE_CYCLE", error.message)
                    })?;
            }
        }
        if let Some(children) = crate::operations::object_refs(original) {
            if children.iter().any(|child| child.id.value() == from) {
                crate::pool_validation::validate_parent_child_relationship_for_version(
                    after, owner, target, version,
                )
                .map_err(|error| ComponentError::new("REPLACE_CHILD_TYPE", error.message))?;
                crate::pool_validation::would_create_circular_reference(before, owner, target)
                    .map_err(|error| {
                        ComponentError::new("REPLACE_REFERENCE_CYCLE", error.message)
                    })?;
            }
        }
        if let Some(list) = crate::operations::object_list(original) {
            let affected = match list {
                crate::operations::ObjectReferenceList::Required(items) => {
                    items.iter().any(|id| id.value() == from)
                }
                crate::operations::ObjectReferenceList::Nullable(items) => items
                    .iter()
                    .filter_map(|reference| reference.0)
                    .any(|id| id.value() == from),
            };
            if affected {
                crate::pool_validation::validate_parent_child_relationship_for_version(
                    after, owner, target, version,
                )
                .map_err(|error| ComponentError::new("REPLACE_CHILD_TYPE", error.message))?;
                crate::pool_validation::would_create_circular_reference(before, owner, target)
                    .map_err(|error| {
                        ComponentError::new("REPLACE_REFERENCE_CYCLE", error.message)
                    })?;
            }
        }
        if crate::operations::macro_refs(original).is_some_and(|refs| {
            refs.iter()
                .any(|reference| u16::from(reference.macro_id) == from)
        }) && after.object_by_id(target).is_none_or(|object| {
            object.object_type() != ag_iso_stack::object_pool::ObjectType::Macro
        }) {
            return Err(ComponentError::new(
                "REPLACE_MACRO_TYPE",
                "Macro event references must target Macro objects",
            ));
        }
        if let ag_iso_stack::object_pool::object::Object::ObjectLabelReferenceList(labels) =
            original
        {
            for label in &labels.object_labels {
                let references = [
                    Some(label.id),
                    label.string_variable_reference.0,
                    label.graphic_representation.0,
                ];
                if references
                    .into_iter()
                    .any(|id| id.is_some_and(|id| id.value() == from))
                {
                    crate::pool_validation::validate_parent_child_relationship_for_version(
                        after, owner, target, version,
                    )
                    .map_err(|error| ComponentError::new("REPLACE_LABEL_TYPE", error.message))?;
                    crate::pool_validation::would_create_circular_reference(before, owner, target)
                        .map_err(|error| {
                            ComponentError::new("REPLACE_LABEL_CYCLE", error.message)
                        })?;
                }
            }
        }
    }
    Ok(())
}

/// Replace one primitive occurrence, retaining its parent-list slot and offset.
pub fn replace_placement(
    project: &mut EditorProject,
    target: &PlacementTarget,
    to: u16,
) -> Result<(), String> {
    let PlacementTarget::Primitive {
        parent_id,
        child_index,
    } = target
    else {
        return Err("This component placement cannot be replaced directly".into());
    };
    let parent = project
        .get_pool()
        .object_by_id(object_id(*parent_id)?)
        .ok_or_else(|| format!("Parent object {parent_id} no longer exists"))?;
    let children = crate::operations::object_refs(parent)
        .ok_or_else(|| "The selected parent does not support child placements".to_owned())?;
    if *child_index >= children.len() {
        return Err("The selected placement is stale".into());
    }
    let placements = children
        .iter()
        .enumerate()
        .map(|(index, child)| crate::ChildPlacement {
            child: crate::ObjectTarget::id(if index == *child_index {
                to
            } else {
                child.id.value()
            }),
            x: child.offset.x,
            y: child.offset.y,
        })
        .collect();
    let mut transaction = OperationTransaction::new(Some("Replace placement".into()));
    transaction.add_operation(Operation::SetChildren {
        parent: crate::ObjectTarget::id(*parent_id),
        children: placements,
    });
    project
        .execute_transaction(transaction)
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn object_id(id: u16) -> Result<ag_iso_stack::object_pool::ObjectId, String> {
    ag_iso_stack::object_pool::ObjectId::new(id)
        .map_err(|error| format!("Invalid object ID {id}: {error:?}"))
}

fn is_target(target: &crate::ObjectTarget, id: u16, name: &str) -> bool {
    match target {
        crate::ObjectTarget::Id { id: target_id } => *target_id == id,
        crate::ObjectTarget::Name { name: target_name } => {
            crate::smart_naming::names_equal(target_name, name)
        }
    }
}

fn redirect_component_source(
    source: &mut ComponentSource,
    pool: &ag_iso_stack::object_pool::ObjectPool,
    from: u16,
    to: u16,
    from_name: &str,
    version: VtVersion,
) -> ComponentResult<()> {
    let mut types: HashMap<_, _> = pool
        .objects()
        .iter()
        .map(|object| (object.id().value(), object.object_type()))
        .collect();
    for instance in &source.instances {
        if let Some(definition) = source.definitions.get(&instance.component_id) {
            for node in &definition.nodes {
                if let Some(id) = instance.compiled_ids.get(&node.id) {
                    types.insert(*id, node.object_type);
                }
            }
        }
    }
    let replace_target = |target: &mut crate::ObjectTarget| {
        if is_target(target, from, from_name) {
            *target = crate::ObjectTarget::id(to);
        }
    };
    for definition in source.definitions.values_mut() {
        for node in &mut definition.nodes {
            if node.object_type == ag_iso_stack::object_pool::ObjectType::Macro {
                if let Some(value) = node
                    .properties
                    .get_mut(&crate::object_properties::PropertyKey::Commands)
                {
                    let commands: Vec<u8> = serde_json::from_value(value.clone())
                        .map_err(|error| ComponentError::new("REPLACE_MACRO", error.to_string()))?;
                    let rewritten = crate::macro_id_remap::remap_macro_object_ids_checked(
                        &commands,
                        &HashMap::from([(from, to)]),
                        version,
                        &types,
                    )
                    .map_err(|error| ComponentError::new("REPLACE_MACRO", error))?;
                    *value = serde_json::to_value(rewritten)
                        .map_err(|error| ComponentError::new("REPLACE_MACRO", error.to_string()))?;
                }
            }
            for (property, reference) in &mut node.references {
                if node.object_type == ag_iso_stack::object_pool::ObjectType::ExternalObjectPointer
                    && *property == crate::object_properties::PropertyKey::ExternalObjectId
                {
                    continue;
                }
                if let ComponentReference::ExternalObject(target) = reference {
                    replace_target(target);
                }
            }
            for reference in &mut node.macro_references {
                replace_target(&mut reference.target);
            }
            match &mut node.object_list {
                Some(ComponentObjectList::Required(items)) => {
                    items.iter_mut().for_each(replace_target)
                }
                Some(ComponentObjectList::Nullable(items)) => {
                    items.iter_mut().flatten().for_each(replace_target)
                }
                None => {}
            }
        }
        for parameter in &mut definition.parameters {
            if parameter.kind == ComponentParameterKind::ObjectReference {
                redirect_target_value(&mut parameter.default_value, from, to, from_name);
            }
        }
    }
    for instance in &mut source.instances {
        if let Some(definition) = source.definitions.get(&instance.component_id) {
            for parameter in &definition.parameters {
                if parameter.kind == ComponentParameterKind::ObjectReference {
                    if let Some(value) = instance.overrides.get_mut(&parameter.id) {
                        redirect_target_value(value, from, to, from_name);
                    }
                }
            }
        }
    }
    let mut candidate_types = types;
    for object in pool.objects() {
        candidate_types.insert(object.id().value(), object.object_type());
    }
    for definition in source.definitions.values() {
        for node in &definition.nodes {
            if node.object_type == ag_iso_stack::object_pool::ObjectType::Macro {
                if let Some(value) = node
                    .properties
                    .get(&crate::object_properties::PropertyKey::Commands)
                {
                    let commands: Vec<u8> = serde_json::from_value(value.clone())
                        .map_err(|error| ComponentError::new("REPLACE_MACRO", error.to_string()))?;
                    crate::macro_id_remap::validate_macro_reference_values(
                        &commands,
                        version,
                        &candidate_types,
                    )
                    .map_err(|error| ComponentError::new("REPLACE_MACRO", error))?;
                }
            }
        }
    }
    Ok(())
}

fn redirect_target_value(value: &mut serde_json::Value, from: u16, to: u16, from_name: &str) {
    if value.as_u64() == Some(u64::from(from)) {
        *value = serde_json::Value::from(to);
    } else if let Ok(mut target) = serde_json::from_value::<crate::ObjectTarget>(value.clone()) {
        if is_target(&target, from, from_name) {
            target = crate::ObjectTarget::id(to);
            if let Ok(rewritten) = serde_json::to_value(target) {
                *value = rewritten;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ag_iso_stack::object_pool::{object::Object, ObjectPool, ObjectType};

    fn object_with_id(object_type: ObjectType, id: u16) -> Object {
        let mut object = crate::default_object(object_type);
        object.mut_id().set_value(id).unwrap();
        object
    }

    fn fixture() -> EditorProject {
        let mut pool = ObjectPool::new();
        pool.add(object_with_id(ObjectType::StringVariable, 10));
        pool.add(object_with_id(ObjectType::StringVariable, 11));
        pool.add(object_with_id(ObjectType::NumberVariable, 12));
        pool.add(object_with_id(ObjectType::FontAttributes, 30));
        let mut output = object_with_id(ObjectType::OutputString, 20);
        if let Object::OutputString(output) = &mut output {
            output.font_attributes = ag_iso_stack::object_pool::ObjectId::new(30).unwrap().into();
            output.variable_reference = ag_iso_stack::object_pool::NullableObjectId(Some(
                ag_iso_stack::object_pool::ObjectId::new(10).unwrap(),
            ));
        }
        pool.add(output);
        EditorProject::from(pool)
    }

    #[test]
    fn object_wide_replacement_allows_a_compatible_different_type() {
        let mut pool = ObjectPool::new();
        pool.add(object_with_id(ObjectType::StringVariable, 10));
        pool.add(object_with_id(ObjectType::NumberVariable, 12));
        let mut pointer = object_with_id(ObjectType::ObjectPointer, 20);
        if let Object::ObjectPointer(pointer) = &mut pointer {
            pointer.value = ag_iso_stack::object_pool::NullableObjectId(Some(
                ag_iso_stack::object_pool::ObjectId::new(10).unwrap(),
            ));
        }
        pool.add(pointer);
        let mut project = EditorProject::from(pool);
        project.replace_object_references(10, 12).unwrap();
        let Object::ObjectPointer(pointer) = project
            .get_pool()
            .object_by_id(object_id(20).unwrap())
            .unwrap()
        else {
            panic!("expected Object Pointer")
        };
        assert_eq!(pointer.value.0.unwrap().value(), 12);
    }

    #[test]
    fn object_wide_replacement_preserves_definitions_and_is_undoable() {
        let mut project = fixture();
        project.replace_object_references(10, 11).unwrap();
        let output = project
            .get_pool()
            .object_by_id(object_id(20).unwrap())
            .unwrap();
        let Object::OutputString(output) = output else {
            panic!("expected Output String")
        };
        assert_eq!(output.variable_reference.0.unwrap().value(), 11);
        assert!(project
            .get_pool()
            .object_by_id(object_id(10).unwrap())
            .is_some());
        project.undo();
        let Object::OutputString(output) = project
            .get_pool()
            .object_by_id(object_id(20).unwrap())
            .unwrap()
        else {
            panic!("expected Output String")
        };
        assert_eq!(output.variable_reference.0.unwrap().value(), 10);
        project.redo();
        let Object::OutputString(output) = project
            .get_pool()
            .object_by_id(object_id(20).unwrap())
            .unwrap()
        else {
            panic!("expected Output String")
        };
        assert_eq!(output.variable_reference.0.unwrap().value(), 11);
    }

    #[test]
    fn object_wide_replacement_rejects_incompatible_target_atomically() {
        let mut project = fixture();
        let revision = project.revision();
        assert!(project.replace_object_references(10, 12).is_err());
        assert_eq!(project.revision(), revision);
        let Object::OutputString(output) = project
            .get_pool()
            .object_by_id(object_id(20).unwrap())
            .unwrap()
        else {
            panic!("expected Output String")
        };
        assert_eq!(output.variable_reference.0.unwrap().value(), 10);
    }

    #[test]
    fn occurrence_replacement_keeps_position_and_undoes_as_one_edit() {
        let mut pool = ObjectPool::new();
        let mut mask = object_with_id(ObjectType::DataMask, 1);
        if let Object::DataMask(mask) = &mut mask {
            mask.object_refs.push(ag_iso_stack::object_pool::ObjectRef {
                id: object_id(2).unwrap(),
                offset: ag_iso_stack::object_pool::object_attributes::Point { x: 7, y: 9 },
            });
        }
        pool.add(mask);
        pool.add(object_with_id(ObjectType::OutputRectangle, 2));
        pool.add(object_with_id(ObjectType::OutputRectangle, 3));
        let mut project = EditorProject::from(pool);
        let target = PlacementTarget::Primitive {
            parent_id: 1,
            child_index: 0,
        };
        replace_placement(&mut project, &target, 3).unwrap();
        let mask = project
            .get_pool()
            .object_by_id(object_id(1).unwrap())
            .unwrap();
        let Object::DataMask(mask) = mask else {
            panic!("expected Data Mask")
        };
        assert_eq!(mask.object_refs[0].id.value(), 3);
        assert_eq!(mask.object_refs[0].offset.x, 7);
        assert_eq!(mask.object_refs[0].offset.y, 9);
        project.undo();
        let Object::DataMask(mask) = project
            .get_pool()
            .object_by_id(object_id(1).unwrap())
            .unwrap()
        else {
            panic!("expected Data Mask")
        };
        assert_eq!(mask.object_refs[0].id.value(), 2);
    }
}
