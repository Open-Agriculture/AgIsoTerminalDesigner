//! Typed actions accepted by the shared command editor.

/// Maximum number of edits accepted in one atomic command batch.
pub const MAX_EDIT_ACTIONS: usize = 256;

use ag_iso_stack::object_pool::{object_attributes::Event, ObjectType};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::BTreeMap;

/// Error returned while decoding or applying an atomic edit batch.
#[derive(Debug, Clone)]
pub struct EditBatchError {
    pub code: &'static str,
    pub message: String,
}

impl std::fmt::Display for EditBatchError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for EditBatchError {}

/// Decode and apply a batch using the normal transactional project editor.
/// `args` has the shape `{ "actions": [...], "check": false }`. Check-only
/// batches validate their candidate but leave the project and history unchanged.
pub fn execute_batch(
    project: &mut crate::EditorProject,
    args: Value,
) -> Result<Value, EditBatchError> {
    #[derive(Deserialize)]
    #[serde(deny_unknown_fields)]
    struct BatchArgs {
        actions: Vec<EditAction>,
        #[serde(default)]
        check: bool,
    }

    let args: BatchArgs = serde_json::from_value(args).map_err(|error| EditBatchError {
        code: "VALIDATION_FAILED",
        message: error.to_string(),
    })?;
    if args.actions.is_empty() || args.actions.len() > MAX_EDIT_ACTIONS {
        return Err(EditBatchError {
            code: "INVALID_ARGUMENTS",
            message: format!("A batch requires 1..{} edit actions", MAX_EDIT_ACTIONS),
        });
    }

    let before: std::collections::HashSet<_> = project
        .get_pool()
        .objects()
        .iter()
        .map(|object| object.id().value())
        .collect();
    let applied = project
        .edit_batch(
            "CLI edit",
            |scratch| {
                for action in args.actions {
                    apply(scratch, action)?;
                }
                Ok(())
            },
            args.check,
        )
        .map_err(|message| EditBatchError {
            code: "VALIDATION_FAILED",
            message,
        })?;
    let allocated: Vec<_> = project
        .get_pool()
        .objects()
        .iter()
        .map(|object| object.id().value())
        .filter(|id| !before.contains(id))
        .collect();

    Ok(json!({
        "transaction_id": applied.as_ref().map(|transaction| &transaction.transaction_id),
        "changed_ids": applied.as_ref().map(|transaction| transaction.affected_objects.as_slice()).unwrap_or_default(),
        "allocated_ids": allocated,
        "check": args.check,
    }))
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EditAction {
    #[serde(flatten)]
    pub kind: EditActionKind,
    #[serde(default, rename = "_line")]
    pub line: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum EditActionKind {
    Operation {
        operation: Operation,
    },
    Component {
        action: ComponentIntent,
    },
    AuthorComponent {
        action: crate::cli::component_authoring::ComponentAuthoringAction,
    },
    Settings {
        values: BTreeMap<String, String>,
    },
    SetProperties {
        object: ObjectTarget,
        values: BTreeMap<PropertyKey, String>,
    },
    Relationship {
        #[serde(default)]
        object: Option<ObjectTarget>,
        operation: Operation,
    },
    RelationshipEdit {
        edit: RelationshipEdit,
    },
    ObjectDuplicate {
        object_id: u16,
        name: Option<String>,
    },
    ObjectIdAuto {
        object_id: u16,
    },
    ObjectIdReassign {
        #[serde(default)]
        include_overrides: bool,
        #[serde(default)]
        mappings: Option<Vec<(u16, u16)>>,
    },
    ObjectFieldSet {
        object: ObjectTarget,
        property: PropertyKey,
        field: String,
        values: Vec<String>,
    },
    ObjectArraySet {
        object: ObjectTarget,
        property: PropertyKey,
        field: String,
        index: usize,
        values: Vec<String>,
    },
    ObjectArrayAppend {
        object: ObjectTarget,
        property: PropertyKey,
        field: String,
        values: Vec<String>,
    },
    ObjectArrayRemove {
        object: ObjectTarget,
        property: PropertyKey,
        field: String,
        index: usize,
    },
    ObjectArrayClear {
        object: ObjectTarget,
        property: PropertyKey,
        field: String,
    },
    ObjectBytesSet {
        object: ObjectTarget,
        property: PropertyKey,
        #[serde(with = "base64_bytes")]
        bytes: Vec<u8>,
    },
    IdRangeSet {
        object_type: ObjectType,
        start: u16,
        end: u16,
    },
    IdRangeReset,
    ComponentCapture {
        root: u16,
        name: String,
        #[serde(default)]
        keep_external_references: bool,
    },
    ComponentMove {
        instance_id: ComponentInstanceId,
        x: i32,
        y: i32,
    },
    ComponentDelete {
        instance_id: ComponentInstanceId,
    },
    ComponentDuplicate {
        instance_id: ComponentInstanceId,
        x: i32,
        y: i32,
    },
    ComponentDetach {
        instance_id: ComponentInstanceId,
    },
    ComponentDefinitionRename {
        component_id: ComponentId,
        name: String,
    },
    ComponentDefinitionDuplicate {
        component_id: ComponentId,
    },
    ComponentDefinitionDelete {
        component_id: ComponentId,
    },
    ComponentNodeSet {
        component_id: ComponentId,
        node_id: ComponentNodeId,
        property: PropertyKey,
        value: String,
    },
    ComponentNodeReference {
        component_id: ComponentId,
        node_id: ComponentNodeId,
        property: PropertyKey,
        target: String,
    },
    ComponentParameterAdd {
        component_id: ComponentId,
        parameter: ParameterInput,
    },
    ComponentParameterSet {
        component_id: ComponentId,
        parameter: ParameterInput,
    },
    ComponentParameterRemove {
        component_id: ComponentId,
        parameter_id: ComponentParameterId,
    },
    ComponentOverride {
        instance_id: ComponentInstanceId,
        parameter: String,
        value: String,
    },
    ComponentReset {
        instance_id: ComponentInstanceId,
        parameter: String,
    },
    GraphicImport {
        filename: String,
        #[serde(rename = "bytes_base64", with = "base64_bytes")]
        bytes: Vec<u8>,
        #[serde(default)]
        name: Option<String>,
        #[serde(default)]
        asset_id: Option<crate::graphics::GraphicAssetId>,
        #[serde(default)]
        original_file_name: Option<String>,
        #[serde(default)]
        clear_original_name: bool,
        #[serde(default)]
        description: Option<String>,
        #[serde(default)]
        tags: Vec<String>,
    },
    GraphicAssignFile {
        object: u16,
        #[serde(with = "base64_bytes")]
        bytes: Vec<u8>,
        format: crate::graphics::PictureGraphicFormat,
        encoding: crate::graphics::DataCodeType,
    },
    MacroAppend {
        object: ObjectTarget,
        instruction: String,
        #[serde(default)]
        arguments: Vec<String>,
    },
    ObjectMetadataSet {
        object: ObjectTarget,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "nullable_update"
        )]
        name: Option<Option<String>>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "nullable_update"
        )]
        description: Option<Option<String>>,
        #[serde(
            default,
            skip_serializing_if = "Option::is_none",
            deserialize_with = "nullable_update"
        )]
        notes: Option<Option<String>>,
        #[serde(default)]
        tags: Option<Vec<String>>,
        #[serde(default)]
        id_overridden: Option<bool>,
    },
    GraphicAssign {
        asset_id: crate::graphics::GraphicAssetId,
        object: u16,
    },
    GraphicCreatePicture {
        asset_id: crate::graphics::GraphicAssetId,
    },
    GraphicRename {
        asset_id: crate::graphics::GraphicAssetId,
        name: String,
    },
    GraphicDelete {
        asset_id: crate::graphics::GraphicAssetId,
    },
}

// Missing means leave unchanged; an explicit null means clear the field.
fn nullable_update<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}

/// Relationship edits whose values depend on the current project content.
/// Targets remain symbolic until the atomic batch is applied, allowing earlier
/// actions in that batch to create or rename the referenced objects.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RelationshipEdit {
    ListSet {
        object: ObjectTarget,
        required: Option<bool>,
        objects: Vec<Option<ObjectTarget>>,
    },
    ListInsert {
        object: ObjectTarget,
        index: usize,
        target: Option<ObjectTarget>,
    },
    ListRemove {
        object: ObjectTarget,
        index: usize,
    },
    MacroBindings {
        object: ObjectTarget,
        replace: bool,
        bindings: Vec<MacroReferenceTarget>,
    },
    MacroUnbind {
        object: ObjectTarget,
        event: Event,
    },
    MacroOrder {
        object: ObjectTarget,
        events: Vec<Event>,
    },
    LabelSet {
        object: ObjectTarget,
        replace: bool,
        id: ObjectTarget,
        string: Option<ObjectTarget>,
        graphic: Option<ObjectTarget>,
        font: u8,
    },
    LabelRemove {
        object: ObjectTarget,
        id: ObjectTarget,
    },
    LabelClear {
        object: ObjectTarget,
    },
    PlacementMove {
        parent: ObjectTarget,
        child: ObjectTarget,
        x: i16,
        y: i16,
        occurrence: Option<usize>,
    },
    PlacementRemove {
        parent: ObjectTarget,
        child: ObjectTarget,
        occurrence: Option<usize>,
    },
    PlacementOrder {
        parent: ObjectTarget,
        children: Vec<ObjectTarget>,
    },
}

impl EditAction {
    pub fn new(kind: EditActionKind) -> Self {
        Self { kind, line: None }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParameterInput {
    #[serde(default)]
    pub id: Option<ComponentParameterId>,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    pub target_node: ComponentNodeId,
    pub property: ComponentParameterTarget,
    pub kind: ComponentParameterKind,
    #[serde(default)]
    pub default_value: Option<String>,
}

pub(crate) mod base64_bytes {
    use base64::{engine::general_purpose::STANDARD, Engine};
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(bytes: &[u8], serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&STANDARD.encode(bytes))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let encoded = String::deserialize(deserializer)?;
        STANDARD.decode(encoded).map_err(serde::de::Error::custom)
    }
}

use crate::graphics::integration::GraphicService;
use crate::{
    components::{
        plan_component_capture, ComponentDefinition, ComponentId, ComponentInstanceId,
        ComponentIntent, ComponentNodeId, ComponentParameter, ComponentParameterId,
        ComponentParameterKind, ComponentParameterTarget, ComponentService,
    },
    object_info::ObjectInfo,
    object_properties::{
        get_object_property, get_property_descriptors, PropertyKey, PropertySemantic,
    },
    operations::{
        object_list, object_refs, ChildPlacement, MacroReferenceTarget, ObjectReferenceList,
        ObjectTarget, ObjectTargetList, Operation,
    },
    EditorProject,
};
use ag_iso_stack::object_pool::{
    object::Object, object_attributes::ObjectLabel, NullableObjectId, ObjectId,
};
// Dispatch commands through the same services used by the editor.
fn apply_kind(project: &mut EditorProject, action: EditActionKind) -> Result<(), String> {
    use EditActionKind as A;
    match action {
        A::ComponentCapture {
            root,
            name,
            keep_external_references,
        } => component_capture(project, root, &name, keep_external_references),
        A::AuthorComponent { action } => crate::cli::component_authoring::apply(project, action),
        A::ComponentMove { instance_id, x, y } => {
            ComponentService::move_instance(project, instance_id, x, y).map_err(|e| e.to_string())
        }
        A::ComponentDelete { instance_id } => {
            ComponentService::delete_component_instance(project, instance_id)
                .map_err(|e| e.to_string())
        }
        A::ComponentDuplicate { instance_id, x, y } => {
            ComponentService::duplicate_component_instance(project, instance_id, x, y)
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
        A::ComponentDetach { instance_id } => {
            ComponentService::detach_component_instance(project, instance_id)
                .map_err(|e| e.to_string())
        }
        A::ComponentDefinitionRename { component_id, name } => {
            let mut definition = definition(project, component_id)?;
            definition.name = name;
            ComponentService::edit_component_definition(project, definition)
                .map_err(|e| e.to_string())
        }
        A::ComponentDefinitionDuplicate { component_id } => {
            ComponentService::duplicate_definition(project, component_id)
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
        A::ComponentDefinitionDelete { component_id } => {
            ComponentService::delete_definition(project, component_id).map_err(|e| e.to_string())
        }
        A::ComponentNodeSet {
            component_id,
            node_id,
            property,
            value,
        } => component_node_set(project, component_id, node_id, property, &value),
        A::ComponentNodeReference {
            component_id,
            node_id,
            property,
            target,
        } => component_node_reference(project, component_id, node_id, property, &target),
        A::ComponentParameterAdd {
            component_id,
            parameter,
        } => component_parameter(project, component_id, parameter, false),
        A::ComponentParameterSet {
            component_id,
            parameter,
        } => component_parameter(project, component_id, parameter, true),
        A::ComponentParameterRemove {
            component_id,
            parameter_id,
        } => ComponentService::remove_component_parameter(project, component_id, parameter_id)
            .map_err(|e| e.to_string()),
        A::ComponentOverride {
            instance_id,
            parameter,
            value,
        } => component_override(project, instance_id, &parameter, &value),
        A::ComponentReset {
            instance_id,
            parameter,
        } => component_reset(project, instance_id, &parameter),
        A::ObjectDuplicate { object_id, name } => {
            object_duplicate(project, object_id, name.as_deref())
        }
        A::ObjectIdAuto { object_id } => object_id_auto(project, object_id),
        A::ObjectIdReassign {
            include_overrides,
            mappings,
        } => object_id_reassign(project, include_overrides, mappings),
        A::ObjectFieldSet {
            object,
            property,
            field,
            values,
        } => structured_property(
            project,
            object,
            property,
            StructuredEdit::Field(field, values),
        ),
        A::ObjectArraySet {
            object,
            property,
            field,
            index,
            values,
        } => structured_property(
            project,
            object,
            property,
            StructuredEdit::Array(field, ArrayEdit::Set(index, values)),
        ),
        A::ObjectArrayAppend {
            object,
            property,
            field,
            values,
        } => structured_property(
            project,
            object,
            property,
            StructuredEdit::Array(field, ArrayEdit::Append(values)),
        ),
        A::ObjectArrayRemove {
            object,
            property,
            field,
            index,
        } => structured_property(
            project,
            object,
            property,
            StructuredEdit::Array(field, ArrayEdit::Remove(index)),
        ),
        A::ObjectArrayClear {
            object,
            property,
            field,
        } => structured_property(
            project,
            object,
            property,
            StructuredEdit::Array(field, ArrayEdit::Clear),
        ),
        A::ObjectBytesSet {
            object,
            property,
            bytes,
        } => object_bytes_set(project, object, property, bytes),
        A::RelationshipEdit { edit } => relationship_edit(project, edit),
        A::Relationship { object, operation } => apply_relationship(project, object, operation),
        A::IdRangeSet {
            object_type,
            start,
            end,
        } => {
            let mut ranges = project.object_id_ranges().clone();
            ranges.set_range(object_type, crate::ObjectIdRange { start, end });
            project.set_object_id_ranges(ranges)
        }
        A::IdRangeReset => project.reset_object_id_ranges(),
        A::Operation { operation: op } => operation(project, &op),
        A::Component { action } => {
            ComponentService::apply_intents(project, &[action]).map_err(|e| e.to_string())
        }
        A::Settings { values } => settings(project, values),
        A::SetProperties { object, values } => set_properties(project, object, values),
        A::GraphicImport {
            filename,
            bytes,
            name,
            asset_id,
            original_file_name,
            clear_original_name,
            description,
            tags,
        } => GraphicService::upload_with_metadata(
            project,
            &filename,
            bytes,
            asset_id.unwrap_or_default(),
            name,
            original_file_name,
            clear_original_name,
            description,
            Some(tags),
        )
        .map(|_| ())
        .map_err(|e| e.to_string()),
        A::GraphicAssignFile {
            object,
            bytes,
            format,
            encoding,
        } => GraphicService::assign_indexed_png(project, object, &bytes, format, encoding)
            .map_err(|e| e.to_string()),
        A::MacroAppend {
            object,
            instruction,
            arguments,
        } => macro_append(project, &object, &instruction, &arguments),
        A::ObjectMetadataSet {
            object,
            name,
            description,
            notes,
            tags,
            id_overridden,
        } => object_metadata_set(
            project,
            &object,
            name,
            description,
            notes,
            tags,
            id_overridden,
        ),
        A::GraphicAssign { asset_id, object } => {
            GraphicService::convert_asset_into_picture_graphic(project, asset_id, object)
                .map_err(|e| e.to_string())
        }
        A::GraphicCreatePicture { asset_id } => {
            GraphicService::create_picture_graphic_from_asset(project, asset_id)
                .map(|_| ())
                .map_err(|e| e.to_string())
        }
        A::GraphicRename { asset_id, name } => {
            GraphicService::rename_available_asset(project, asset_id, name)
                .map_err(|e| e.to_string())
        }
        A::GraphicDelete { asset_id } => {
            GraphicService::delete_available_asset(project, asset_id).map_err(|e| e.to_string())
        }
    }
}

fn component_node_set(
    project: &mut EditorProject,
    component_id: ComponentId,
    node_id: ComponentNodeId,
    property: PropertyKey,
    lexical: &str,
) -> Result<(), String> {
    let definition = definition(project, component_id)?;
    let node = definition
        .nodes
        .iter()
        .find(|node| node.id == node_id)
        .ok_or_else(|| format!("Component node {node_id} not found"))?;
    let value = parse_node_property_value(project, node, property, lexical)?;
    ComponentService::apply_intents(
        project,
        &[
            crate::components::ComponentIntent::SetComponentMasterProperty {
                component_id,
                node_id,
                property,
                value,
                scope: crate::components::ComponentMasterScope::AllInstances,
            },
        ],
    )
    .map_err(|e| e.to_string())
}

fn component_node_reference(
    project: &mut EditorProject,
    component_id: ComponentId,
    node_id: ComponentNodeId,
    property: PropertyKey,
    target: &str,
) -> Result<(), String> {
    use crate::components::ComponentReference;
    let mut definition = definition(project, component_id)?;
    let node = definition
        .nodes
        .iter()
        .find(|node| node.id == node_id)
        .ok_or("Unknown component node")?;
    let descriptor = get_property_descriptors(node.object_type)
        .into_iter()
        .find(|descriptor| descriptor.name == property)
        .ok_or("Unknown node property")?;
    if !descriptor.writable
        || !matches!(
            descriptor.semantic,
            PropertySemantic::ObjectReference { .. }
        )
    {
        return Err(format!("'{property}' is not a writable reference"));
    }
    let reference = if target == "null" {
        ComponentReference::Null
    } else if let Some(id) = target.strip_prefix("node:") {
        let id = ComponentNodeId(uuid::Uuid::parse_str(id).map_err(|e| e.to_string())?);
        if !definition.nodes.iter().any(|node| node.id == id) {
            return Err("Unknown internal target node".into());
        }
        ComponentReference::Internal(id)
    } else {
        ComponentReference::ExternalObject(object_target(project, target)?)
    };
    let bound = definition.parameters.iter_mut().find(|parameter| {
        parameter.target_node == node_id
            && parameter.property == ComponentParameterTarget::Property(property)
    });
    let authored = if let Some(parameter) = bound {
        parameter.default_value = match &reference {
            ComponentReference::Null => Value::Null,
            ComponentReference::ExternalObject(target) => {
                serde_json::to_value(target).map_err(|e| e.to_string())?
            }
            _ => {
                return Err(
                    "Remove the reference parameter before binding an internal node".into(),
                );
            }
        };
        ComponentReference::Parameter(parameter.id)
    } else {
        reference
    };
    let node = definition
        .nodes
        .iter_mut()
        .find(|node| node.id == node_id)
        .unwrap();
    node.properties.remove(&property);
    node.references.insert(property, authored);
    ComponentService::edit_component_definition(project, definition).map_err(|e| e.to_string())
}

fn component_capture(
    project: &mut EditorProject,
    root: u16,
    name: &str,
    keep_external_references: bool,
) -> Result<(), String> {
    let plan = plan_component_capture(project, root, name.to_owned()).map_err(|e| e.to_string())?;
    ComponentService::create_component_from_selection(project, plan, keep_external_references)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn component_override(
    project: &mut EditorProject,
    instance_id: ComponentInstanceId,
    parameter_key: &str,
    lexical: &str,
) -> Result<(), String> {
    let source = project.component_source();
    let instance = source
        .instances
        .iter()
        .find(|instance| instance.id == instance_id)
        .ok_or_else(|| format!("Component instance {instance_id} not found"))?;
    let definition = source
        .definitions
        .get(&instance.component_id)
        .ok_or_else(|| format!("Component {} not found", instance.component_id))?;
    let parameter = find_parameter(definition, parameter_key)?;
    let value = {
        let node = definition
            .nodes
            .iter()
            .find(|node| node.id == parameter.target_node)
            .ok_or_else(|| format!("Component node {} not found", parameter.target_node))?;
        coerce_parameter_value(project, node, parameter.property, parameter.kind, lexical)?
    };
    ComponentService::set_component_override(project, instance_id, parameter.id, value)
        .map_err(|e| e.to_string())
}

fn component_reset(
    project: &mut EditorProject,
    instance_id: ComponentInstanceId,
    parameter_key: &str,
) -> Result<(), String> {
    let source = project.component_source();
    let instance = source
        .instances
        .iter()
        .find(|instance| instance.id == instance_id)
        .ok_or_else(|| format!("Component instance {instance_id} not found"))?;
    let definition = source
        .definitions
        .get(&instance.component_id)
        .ok_or_else(|| format!("Component {} not found", instance.component_id))?;
    let parameter = find_parameter(definition, parameter_key)?;
    ComponentService::reset_component_override(project, instance_id, parameter.id)
        .map_err(|e| e.to_string())
}

fn macro_append(
    project: &mut EditorProject,
    target: &ObjectTarget,
    instruction: &str,
    arguments: &[String],
) -> Result<(), String> {
    let object_id = resolve_object(project, target)?;
    let encoded = crate::macro_authoring::encode_macro_instruction(
        instruction,
        arguments,
        project.vt_version,
    )?;
    let version = project.vt_version;
    project
        .edit_source("Append Macro instruction", |pool, _, _, _| {
            let id = ObjectId::new(object_id).map_err(|_| {
                crate::components::ComponentError::new("OBJECT_NOT_FOUND", "Invalid object ID")
            })?;
            let Some(Object::Macro(macro_object)) = pool.object_mut_by_id(id) else {
                return Err(crate::components::ComponentError::new(
                    "INVALID_OBJECT_TYPE",
                    format!("Object {object_id} is not a Macro"),
                ));
            };
            let mut commands = macro_object.commands.clone();
            commands.extend_from_slice(&encoded);
            crate::vt_macro_commands::validate_macro_commands(&commands, version).map_err(
                |error| crate::components::ComponentError::new("INVALID_MACRO_COMMANDS", error),
            )?;
            macro_object.commands = commands;
            Ok(())
        })
        .map_err(|error| error.to_string())
}

fn object_metadata_set(
    project: &mut EditorProject,
    target: &ObjectTarget,
    name: Option<Option<String>>,
    description: Option<Option<String>>,
    notes: Option<Option<String>>,
    tags: Option<Vec<String>>,
    id_overridden: Option<bool>,
) -> Result<(), String> {
    let object_id = resolve_object(project, target)?;
    project
        .edit_source("Edit object metadata", |pool, _, _, object_info| {
            let id = ObjectId::new(object_id).map_err(|_| {
                crate::components::ComponentError::new("OBJECT_NOT_FOUND", "Invalid object ID")
            })?;
            let object = pool.object_by_id(id).ok_or_else(|| {
                crate::components::ComponentError::new(
                    "OBJECT_NOT_FOUND",
                    format!("Object {object_id} not found"),
                )
            })?;
            let info = object_info
                .entry(id)
                .or_insert_with(|| ObjectInfo::new(object));
            if let Some(name) = name {
                info.name = name;
            }
            if let Some(description) = description {
                info.description = description;
            }
            if let Some(notes) = notes {
                info.notes = notes;
            }
            if let Some(tags) = tags {
                info.tags = tags;
            }
            if let Some(overridden) = id_overridden {
                info.id_overridden = overridden;
            }
            Ok(())
        })
        .map_err(|error| error.to_string())
}

fn component_parameter(
    project: &mut EditorProject,
    component_id: ComponentId,
    input: ParameterInput,
    edit: bool,
) -> Result<(), String> {
    let lexical = input.default_value.clone();
    let mut parameter = ComponentParameter {
        id: input.id.unwrap_or_default(),
        name: input.name,
        description: input.description,
        target_node: input.target_node,
        property: input.property,
        kind: input.kind,
        default_value: Value::Null,
    };
    if let Some(lexical) = lexical {
        let definition = definition(project, component_id)?;
        let node = definition
            .nodes
            .iter()
            .find(|node| node.id == parameter.target_node)
            .ok_or_else(|| format!("Component node {} not found", parameter.target_node))?;
        parameter.default_value =
            coerce_parameter_value(project, node, parameter.property, parameter.kind, &lexical)?;
    }
    if edit {
        ComponentService::edit_component_parameter(project, component_id, parameter)
    } else {
        ComponentService::add_component_parameter(project, component_id, parameter)
    }
    .map_err(|e| e.to_string())
}

fn find_parameter<'a>(
    definition: &'a ComponentDefinition,
    token: &str,
) -> Result<&'a ComponentParameter, String> {
    if let Ok(id) = uuid::Uuid::parse_str(token).map(ComponentParameterId) {
        if let Some(parameter) = definition
            .parameters
            .iter()
            .find(|parameter| parameter.id == id)
        {
            return Ok(parameter);
        }
    }
    let matches: Vec<_> = definition
        .parameters
        .iter()
        .filter(|parameter| parameter.name == token)
        .collect();
    match matches.as_slice() {
        [parameter] => Ok(parameter),
        [] => Err(format!("Unknown component parameter '{token}'")),
        _ => Err(format!("Ambiguous component parameter '{token}'")),
    }
}

fn coerce_parameter_value(
    project: &EditorProject,
    node: &crate::components::ComponentNode,
    target: ComponentParameterTarget,
    kind: ComponentParameterKind,
    lexical: &str,
) -> Result<Value, String> {
    match kind {
        ComponentParameterKind::Text => Ok(Value::String(lexical.to_owned())),
        ComponentParameterKind::Number => parse_number(lexical),
        ComponentParameterKind::Boolean => match lexical {
            "true" => Ok(Value::Bool(true)),
            "false" => Ok(Value::Bool(false)),
            _ => Err(format!("'{lexical}' must be true or false")),
        },
        ComponentParameterKind::Graphic => lexical
            .parse::<u16>()
            .map(|id| json!(id))
            .map_err(|_| format!("'{lexical}' must be a graphic ID")),
        ComponentParameterKind::ObjectReference => {
            if lexical == "null" {
                Ok(Value::Null)
            } else {
                serde_json::to_value(object_target(project, lexical)?)
                    .map_err(|e| format!("Invalid object reference: {e}"))
            }
        }
        ComponentParameterKind::Colour => colour_value(project, lexical),
        ComponentParameterKind::Enum => match target {
            ComponentParameterTarget::Property(property) => {
                parse_node_property_value(project, node, property, lexical)
            }
            ComponentParameterTarget::Graphic => {
                Err("Graphic parameters do not accept enum values".into())
            }
        },
    }
}

fn parse_node_property_value(
    project: &EditorProject,
    node: &crate::components::ComponentNode,
    property: PropertyKey,
    lexical: &str,
) -> Result<Value, String> {
    let prototype = node
        .properties
        .get(&property)
        .cloned()
        .or_else(|| get_object_property(&crate::default_object(node.object_type), property).ok())
        .ok_or_else(|| format!("No schema for '{property}'"))?;
    parse_property_value(project, node.object_type, property, &prototype, lexical)
}

fn object_duplicate(
    project: &mut EditorProject,
    source_id: u16,
    requested_name: Option<&str>,
) -> Result<(), String> {
    let source = project
        .get_pool()
        .object_by_id(ObjectId::new(source_id).map_err(|e| format!("Invalid object ID: {e:?}"))?)
        .ok_or_else(|| format!("Object {source_id} not found"))?
        .clone();
    let object_type = source.object_type();
    let id = project
        .allocate_object_id(object_type)
        .map_err(|e| e.to_string())?
        .value();
    let source_info = project.get_object_info(&source);
    let base_name = requested_name
        .map(str::to_owned)
        .unwrap_or_else(|| format!("Copy of {}", source_info.get_name(&source)));
    if base_name.trim().is_empty() {
        return Err("Duplicate object name cannot be empty".into());
    }
    let names: std::collections::HashSet<_> = project
        .get_pool()
        .objects()
        .iter()
        .map(|object| project.get_object_info(object).get_name(object))
        .collect();
    let mut name = base_name.clone();
    let mut suffix = 2_u32;
    while names.contains(&name) {
        name = format!("{base_name} ({suffix})");
        suffix += 1;
    }
    let id = ObjectId::new(id).map_err(|e| format!("Invalid object ID: {e:?}"))?;
    let mut duplicate = source;
    duplicate
        .mut_id()
        .set_value(id.value())
        .map_err(|e| format!("Invalid duplicate object ID: {e:?}"))?;
    let mut info = ObjectInfo::new(&duplicate);
    info.name = Some(name);
    info.description = source_info.description;
    info.notes = source_info.notes;
    info.tags = source_info.tags;
    info.allowed_references = source_info.allowed_references;
    operation(
        project,
        &Operation::RestoreObject {
            object: duplicate,
            info: Some(info),
        },
    )?;
    Ok(())
}

fn object_id_auto(project: &mut EditorProject, old_id: u16) -> Result<(), String> {
    let id = ObjectId::new(old_id).map_err(|e| format!("Invalid object ID: {e:?}"))?;
    let new_id = project.queue_use_automatic_object_id(id)?;
    // Drain the UI queue through its normal commit path.
    if !project.commit_queued("Use automatic object ID") && new_id != id {
        return Err("Could not assign automatic object ID".into());
    }
    Ok(())
}

fn object_id_reassign(
    project: &mut EditorProject,
    include_overrides: bool,
    mappings: Option<Vec<(u16, u16)>>,
) -> Result<(), String> {
    let plan = project.plan_id_reassignment(include_overrides)?;
    if let Some(supplied) = mappings {
        if supplied != plan.mappings {
            return Err("ID reassignment plan changed; inspect the current plan and retry".into());
        }
    }
    if plan.mappings.is_empty() {
        return Ok(());
    }
    operation(
        project,
        &Operation::RemapObjectIds {
            mappings: plan.mappings,
            target_types: plan.target_types,
        },
    )
}

enum StructuredEdit {
    Field(String, Vec<String>),
    Array(String, ArrayEdit),
}

enum ArrayEdit {
    Set(usize, Vec<String>),
    Append(Vec<String>),
    Remove(usize),
    Clear,
}

fn structured_property(
    project: &mut EditorProject,
    object_target: ObjectTarget,
    property: PropertyKey,
    edit: StructuredEdit,
) -> Result<(), String> {
    let object_id = resolve_object(project, &object_target)?;
    let object = project
        .get_pool()
        .object_by_id(ObjectId::new(object_id).map_err(|e| format!("Invalid object ID: {e:?}"))?)
        .ok_or_else(|| format!("Object {object_id} not found"))?;
    let descriptors = get_property_descriptors(object.object_type());
    let descriptor = descriptors
        .iter()
        .find(|descriptor| descriptor.name == property)
        .ok_or_else(|| format!("Unknown property '{property}'"))?;
    if !descriptor.writable {
        return Err(format!("Property '{property}' is read-only"));
    }
    let mut value = get_object_property(object, property).map_err(|e| e.to_string())?;
    match edit {
        StructuredEdit::Field(path, assignments) => {
            let slot = get_path(&value, &path)?;
            let replacement = edit_value(slot, &assignments, false)?;
            set_path(&mut value, &path, replacement)?;
        }
        StructuredEdit::Array(path, edit) => {
            let mut array_value = get_path(&value, &path)?.clone();
            edit_array(&mut array_value, property, &path, edit)?;
            set_path(&mut value, &path, array_value)?;
        }
    }

    operation(
        project,
        &Operation::SetProperty {
            object: ObjectTarget::id(object_id),
            property,
            value,
        },
    )
}

fn object_bytes_set(
    project: &mut EditorProject,
    object_target: ObjectTarget,
    property: PropertyKey,
    bytes: Vec<u8>,
) -> Result<(), String> {
    let object_id = resolve_object(project, &object_target)?;
    let value = Value::Array(bytes.into_iter().map(|byte| json!(byte)).collect());
    operation(
        project,
        &Operation::SetProperty {
            object: ObjectTarget::id(object_id),
            property,
            value,
        },
    )
}

fn apply_relationship(
    project: &mut EditorProject,
    target: Option<ObjectTarget>,
    mut op: Operation,
) -> Result<(), String> {
    if let Operation::DeleteObject { object_id } = &mut op {
        if let Some(target) = target {
            *object_id = resolve_object(project, &target)?;
        }
    }
    operation(project, &op)
}

fn relationship_edit(project: &mut EditorProject, edit: RelationshipEdit) -> Result<(), String> {
    use RelationshipEdit as R;
    match edit {
        R::ListSet {
            object,
            required,
            objects,
        } => {
            let list = current_object_list(project, resolve_object(project, &object)?)?;
            if required.is_some_and(|required| {
                required != matches!(list, ObjectReferenceList::Required(_))
            }) {
                return Err("Requested list kind does not match this object's list type".into());
            }
            apply_object_list(project, object, list, objects)
        }
        R::ListInsert {
            object,
            index,
            target,
        } => {
            let list = current_object_list(project, resolve_object(project, &object)?)?;
            let mut targets = list_targets(&list);
            if index > targets.len() {
                return Err("List index is out of bounds".into());
            }
            targets.insert(index, target);
            apply_object_list(project, object, list, targets)
        }
        R::ListRemove { object, index } => {
            let list = current_object_list(project, resolve_object(project, &object)?)?;
            let mut targets = list_targets(&list);
            if index >= targets.len() {
                return Err("List index is out of bounds".into());
            }
            targets.remove(index);
            apply_object_list(project, object, list, targets)
        }
        R::MacroBindings {
            object,
            replace,
            bindings: replacements,
        } => {
            if replacements.iter().enumerate().any(|(i, binding)| {
                binding.event_id == Event::Reserved
                    || replacements[..i]
                        .iter()
                        .any(|previous| previous.event_id == binding.event_id)
            }) {
                return Err("Macro bindings require unique, non-reserved events".into());
            }
            let mut bindings = current_macro_refs(project, resolve_object(project, &object)?)?;
            if replace {
                bindings = replacements;
            } else {
                for replacement in replacements {
                    if let Some(current) = bindings
                        .iter_mut()
                        .find(|current| current.event_id == replacement.event_id)
                    {
                        current.macro_object = replacement.macro_object;
                    } else {
                        bindings.push(replacement);
                    }
                }
            }
            operation(
                project,
                &Operation::SetMacroReferences {
                    object,
                    macro_refs: bindings,
                },
            )
        }
        R::MacroUnbind { object, event } => {
            let mut bindings = current_macro_refs(project, resolve_object(project, &object)?)?;
            let len = bindings.len();
            bindings.retain(|binding| binding.event_id != event);
            if bindings.len() == len {
                return Err(format!("No macro is bound to event {event:?}"));
            }
            operation(
                project,
                &Operation::SetMacroReferences {
                    object,
                    macro_refs: bindings,
                },
            )
        }
        R::MacroOrder { object, events } => {
            let bindings = current_macro_refs(project, resolve_object(project, &object)?)?;
            if events.len() != bindings.len()
                || events
                    .iter()
                    .enumerate()
                    .any(|(i, event)| events[..i].contains(event))
            {
                return Err("Event order must list every bound event exactly once".into());
            }
            let ordered = events
                .into_iter()
                .map(|event| {
                    bindings
                        .iter()
                        .find(|binding| binding.event_id == event)
                        .cloned()
                        .ok_or_else(|| format!("Event {event:?} is not currently bound"))
                })
                .collect::<Result<Vec<_>, _>>()?;
            operation(
                project,
                &Operation::SetMacroReferences {
                    object,
                    macro_refs: ordered,
                },
            )
        }
        R::LabelSet {
            object,
            replace,
            id,
            string,
            graphic,
            font,
        } => {
            let object_id = resolve_object(project, &object)?;
            let mut labels = current_labels(project, object_id)?;
            let label = ObjectLabel {
                id: ObjectId::new(resolve_object(project, &id)?)
                    .map_err(|e| format!("Invalid label ID: {e:?}"))?,
                string_variable_reference: NullableObjectId(
                    string
                        .map(|target| {
                            resolve_object(project, &target).and_then(|id| {
                                ObjectId::new(id).map_err(|e| format!("Invalid string ID: {e:?}"))
                            })
                        })
                        .transpose()?,
                ),
                graphic_representation: NullableObjectId(
                    graphic
                        .map(|target| {
                            resolve_object(project, &target).and_then(|id| {
                                ObjectId::new(id).map_err(|e| format!("Invalid graphic ID: {e:?}"))
                            })
                        })
                        .transpose()?,
                ),
                font_type: font,
            };
            if let Some(existing) = labels.iter_mut().find(|existing| existing.id == label.id) {
                if !replace {
                    return Err(format!("Label {} already exists", label.id.value()));
                }
                *existing = label;
            } else {
                labels.push(label);
            }
            operation(project, &Operation::SetObjectLabels { object_id, labels })
        }
        R::LabelRemove { object, id } => {
            let object_id = resolve_object(project, &object)?;
            let mut labels = current_labels(project, object_id)?;
            let id = resolve_object(project, &id)?;
            let len = labels.len();
            labels.retain(|label| label.id.value() != id);
            if labels.len() == len {
                return Err(format!("Label {id} not found"));
            }
            operation(project, &Operation::SetObjectLabels { object_id, labels })
        }
        R::LabelClear { object } => {
            let object_id = resolve_object(project, &object)?;
            current_labels(project, object_id)?;
            operation(
                project,
                &Operation::SetObjectLabels {
                    object_id,
                    labels: Vec::new(),
                },
            )
        }
        R::PlacementMove {
            parent,
            child,
            x,
            y,
            occurrence,
        } => update_placement(project, parent, child, occurrence, Some((x, y))),
        R::PlacementRemove {
            parent,
            child,
            occurrence,
        } => update_placement(project, parent, child, occurrence, None),
        R::PlacementOrder { parent, children } => {
            let parent_id = resolve_object(project, &parent)?;
            let mut remaining = current_placements(project, parent_id)?;
            if children.len() != remaining.len() {
                return Err("placement order must list every child occurrence exactly once".into());
            }
            let mut ordered = Vec::with_capacity(children.len());
            for child in children {
                let id = resolve_object(project, &child)?;
                let index = remaining
                    .iter()
                    .position(|placement| placement.child == ObjectTarget::id(id))
                    .ok_or_else(|| {
                        format!("Child {id} is missing or repeated in placement order")
                    })?;
                ordered.push(remaining.remove(index));
            }
            operation(
                project,
                &Operation::SetChildren {
                    parent,
                    children: ordered,
                },
            )
        }
    }
}

fn update_placement(
    project: &mut EditorProject,
    parent: ObjectTarget,
    child: ObjectTarget,
    occurrence: Option<usize>,
    position: Option<(i16, i16)>,
) -> Result<(), String> {
    let parent_id = resolve_object(project, &parent)?;
    let child_id = resolve_object(project, &child)?;
    let mut placements = current_placements(project, parent_id)?;
    let index = find_placement_index(&placements, child_id, occurrence)?;
    if let Some((x, y)) = position {
        placements[index].x = x;
        placements[index].y = y;
    } else {
        placements.remove(index);
    }
    operation(
        project,
        &Operation::SetChildren {
            parent,
            children: placements,
        },
    )
}

fn current_object_list(
    project: &EditorProject,
    object_id: u16,
) -> Result<ObjectReferenceList, String> {
    let id = ObjectId::new(object_id).map_err(|e| format!("Invalid object ID: {e:?}"))?;
    let object = project
        .get_pool()
        .object_by_id(id)
        .ok_or_else(|| format!("Object {object_id} not found"))?;
    object_list(object)
        .ok_or_else(|| format!("Object {object_id} does not own an ordered object list"))
}

fn list_targets(list: &ObjectReferenceList) -> Vec<Option<ObjectTarget>> {
    match list {
        ObjectReferenceList::Required(ids) => ids
            .iter()
            .map(|id| Some(ObjectTarget::id(id.value())))
            .collect(),
        ObjectReferenceList::Nullable(ids) => ids
            .iter()
            .map(|id| id.0.map(|id| ObjectTarget::id(id.value())))
            .collect(),
    }
}

fn apply_object_list(
    project: &mut EditorProject,
    object: ObjectTarget,
    current: ObjectReferenceList,
    targets: Vec<Option<ObjectTarget>>,
) -> Result<(), String> {
    let objects = match current {
        ObjectReferenceList::Required(_) => {
            let ids = targets
                .into_iter()
                .map(|target| {
                    let target = target.ok_or("Required object lists cannot contain null")?;
                    resolve_object(project, &target).map(ObjectTarget::id)
                })
                .collect::<Result<Vec<_>, _>>()?;
            ObjectTargetList::Required(ids)
        }
        ObjectReferenceList::Nullable(_) => ObjectTargetList::Nullable(
            targets
                .into_iter()
                .map(|target| {
                    target
                        .map(|target| resolve_object(project, &target).map(ObjectTarget::id))
                        .transpose()
                })
                .collect::<Result<Vec<_>, _>>()?,
        ),
    };
    operation(project, &Operation::SetObjectList { object, objects })
}

fn current_macro_refs(
    project: &EditorProject,
    object_id: u16,
) -> Result<Vec<MacroReferenceTarget>, String> {
    let object = project
        .get_pool()
        .object_by_id(ObjectId::new(object_id).map_err(|e| format!("Invalid object ID: {e:?}"))?)
        .ok_or_else(|| format!("Object {object_id} not found"))?;
    crate::operations::macro_refs(object)
        .map(|references| {
            references
                .iter()
                .map(|reference| MacroReferenceTarget {
                    event_id: reference.event_id,
                    macro_object: ObjectTarget::id(reference.macro_id.into()),
                })
                .collect()
        })
        .ok_or_else(|| format!("Object {object_id} does not support macro bindings"))
}

fn current_labels(project: &EditorProject, object_id: u16) -> Result<Vec<ObjectLabel>, String> {
    let object = project
        .get_pool()
        .object_by_id(ObjectId::new(object_id).map_err(|e| format!("Invalid object ID: {e:?}"))?)
        .ok_or_else(|| format!("Object {object_id} not found"))?;
    match object {
        Object::ObjectLabelReferenceList(list) => Ok(list.object_labels.clone()),
        _ => Err(format!(
            "Object {object_id} is not an ObjectLabelReferenceList"
        )),
    }
}

fn current_placements(
    project: &EditorProject,
    parent_id: u16,
) -> Result<Vec<ChildPlacement>, String> {
    let parent = project
        .get_pool()
        .object_by_id(ObjectId::new(parent_id).map_err(|e| format!("Invalid object ID: {e:?}"))?)
        .ok_or_else(|| format!("Parent object {parent_id} not found"))?;
    object_refs(parent)
        .map(|references| {
            references
                .iter()
                .map(|reference| ChildPlacement {
                    child: ObjectTarget::id(reference.id.value()),
                    x: reference.offset.x,
                    y: reference.offset.y,
                })
                .collect()
        })
        .ok_or_else(|| format!("Object {parent_id} does not own positioned children"))
}

fn find_placement_index(
    placements: &[ChildPlacement],
    child_id: u16,
    occurrence: Option<usize>,
) -> Result<usize, String> {
    let matches: Vec<_> = placements
        .iter()
        .enumerate()
        .filter_map(|(index, placement)| {
            (placement.child == ObjectTarget::id(child_id)).then_some(index)
        })
        .collect();
    match occurrence {
        Some(occurrence) => matches.get(occurrence).copied().ok_or_else(|| {
            format!("Placement occurrence {occurrence} not found for child {child_id}")
        }),
        None if matches.len() == 1 => Ok(matches[0]),
        None if matches.is_empty() => {
            Err(format!("Child {child_id} is not placed under this parent"))
        }
        None => Err(format!(
            "Child {child_id} has {} placements; specify --occurrence",
            matches.len()
        )),
    }
}

fn object_target(project: &EditorProject, value: &str) -> Result<ObjectTarget, String> {
    let target = if let Some(name) = value.strip_prefix("name:") {
        ObjectTarget::Name {
            name: name.to_owned(),
        }
    } else if let Some(hex) = value.strip_prefix("0x") {
        ObjectTarget::id(
            u16::from_str_radix(hex, 16).map_err(|_| format!("Invalid object ID '{value}'"))?,
        )
    } else if let Ok(id) = value.parse::<u16>() {
        ObjectTarget::id(id)
    } else {
        ObjectTarget::Name {
            name: value.to_owned(),
        }
    };
    // Resolve names now so callers get useful ambiguity and missing-name errors
    // before the lower-level operation validator reports a generic target error.
    resolve_object(project, &target)?;
    Ok(target)
}

fn set_path(value: &mut Value, path: &str, replacement: Value) -> Result<(), String> {
    if path.is_empty() {
        *value = replacement;
        return Ok(());
    }
    let mut parts = path.split('.').peekable();
    let mut current = value;
    while let Some(part) = parts.next() {
        let is_last = parts.peek().is_none();
        if let Ok(index) = part.parse::<usize>() {
            let array = current
                .as_array_mut()
                .ok_or("Field path expected an array")?;
            current = array.get_mut(index).ok_or("Field index is out of bounds")?;
        } else {
            let object = current
                .as_object_mut()
                .ok_or("Field path expected an object")?;
            if is_last {
                let slot = object
                    .get_mut(part)
                    .ok_or_else(|| format!("Unknown field '{part}'"))?;
                *slot = replacement;
                return Ok(());
            }
            current = object
                .get_mut(part)
                .ok_or_else(|| format!("Unknown field '{part}'"))?;
        }
        if is_last {
            *current = replacement;
            return Ok(());
        }
    }
    Err("Field path cannot be empty".into())
}

fn get_path<'a>(value: &'a Value, path: &str) -> Result<&'a Value, String> {
    if path.is_empty() {
        return Ok(value);
    }
    let mut current = value;
    for part in path.split('.') {
        if let Ok(index) = part.parse::<usize>() {
            current = current
                .as_array()
                .and_then(|array| array.get(index))
                .ok_or("Field index is out of bounds")?;
        } else {
            current = current
                .as_object()
                .and_then(|object| object.get(part))
                .ok_or_else(|| format!("Unknown field '{part}'"))?;
        }
    }
    Ok(current)
}

fn resolve_object(project: &EditorProject, target: &ObjectTarget) -> Result<u16, String> {
    match target {
        ObjectTarget::Id { id } => Ok(*id),
        ObjectTarget::Name { name } => {
            let matches: Vec<_> = project
                .get_pool()
                .objects()
                .iter()
                .filter(|object| project.get_object_info(object).get_name(object) == *name)
                .map(|object| object.id().value())
                .collect();
            match matches.as_slice() {
                [id] => Ok(*id),
                [] => Err(format!("Unknown object name '{name}'")),
                _ => Err(format!(
                    "Ambiguous object name '{name}': {}",
                    matches
                        .iter()
                        .map(u16::to_string)
                        .collect::<Vec<_>>()
                        .join(", ")
                )),
            }
        }
    }
}

fn definition(project: &EditorProject, id: ComponentId) -> Result<ComponentDefinition, String> {
    project
        .component_source()
        .definitions
        .get(&id)
        .cloned()
        .ok_or_else(|| format!("Component {id} not found"))
}

/// Apply one command through the normal editor services.
pub(crate) fn apply(project: &mut EditorProject, action: EditAction) -> Result<(), String> {
    apply_kind(project, action.kind).map_err(|error| match action.line {
        Some(line) => format!("line {line}: {error}"),
        None => error,
    })
}

fn operation(project: &mut EditorProject, op: &Operation) -> Result<(), String> {
    let mut transaction = crate::operations::OperationTransaction::new(Some("CLI edit".into()));
    transaction.add_operation(op.clone());
    project
        .execute_transaction(transaction)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn parse_number(raw: &str) -> Result<Value, String> {
    if let Ok(value) = raw.parse::<i64>() {
        return Ok(json!(value));
    }
    if let Ok(value) = raw.parse::<u64>() {
        return Ok(json!(value));
    }
    raw.parse::<f64>()
        .ok()
        .filter(|n| n.is_finite())
        .map(|n| json!(n))
        .ok_or_else(|| format!("'{raw}' must be a finite number"))
}

fn parse_scalar(prototype: &Value, raw: &str) -> Result<Value, String> {
    match prototype {
        Value::String(_) => Ok(Value::String(raw.into())),
        Value::Bool(_) => raw
            .parse::<bool>()
            .map(Value::Bool)
            .map_err(|_| "Expected true or false".into()),
        Value::Number(n) if n.is_i64() => raw
            .parse::<i64>()
            .map(|n| json!(n))
            .map_err(|_| "Expected an integer".into()),
        Value::Number(n) if n.is_u64() => raw
            .parse::<u64>()
            .map(|n| json!(n))
            .map_err(|_| "Expected a nonnegative integer".into()),
        Value::Number(_) => parse_number(raw),
        _ => Err("Use field assignments or an array command for structured values".into()),
    }
}

fn colour_value(project: &EditorProject, raw: &str) -> Result<Value, String> {
    crate::effective_colour::resolve(project.get_pool(), raw).map(|resolved| json!(resolved.index))
}

fn parse_property_value(
    project: &EditorProject,
    ty: ObjectType,
    property: PropertyKey,
    prototype: &Value,
    raw: &str,
) -> Result<Value, String> {
    let descriptor = get_property_descriptors(ty)
        .into_iter()
        .find(|d| d.name == property)
        .ok_or_else(|| format!("Unknown property '{property}'"))?;
    if !descriptor.writable {
        return Err(format!("Property '{property}' is read-only"));
    }
    match descriptor.semantic {
        PropertySemantic::Colour => colour_value(project, raw),
        PropertySemantic::ObjectReference { .. } => {
            Err("Use the reference command for object references".into())
        }
        PropertySemantic::Enum => descriptor
            .enum_choices
            .iter()
            .find(|choice| {
                choice.as_str() == Some(raw) || (!choice.is_string() && choice.to_string() == raw)
            })
            .cloned()
            .ok_or_else(|| {
                format!(
                    "Invalid '{property}' value '{raw}'; choices: {}",
                    descriptor
                        .enum_choices
                        .iter()
                        .map(|v| v
                            .as_str()
                            .map(str::to_owned)
                            .unwrap_or_else(|| v.to_string()))
                        .collect::<Vec<_>>()
                        .join(", ")
                )
            }),
        _ => parse_scalar(prototype, raw).map_err(|e| format!("{property}: {e}")),
    }
}

fn set_properties(
    project: &mut EditorProject,
    target: ObjectTarget,
    values: BTreeMap<PropertyKey, String>,
) -> Result<(), String> {
    let id = resolve_object(project, &target)?;
    let object = project
        .get_pool()
        .object_by_id(ObjectId::new(id).map_err(|e| format!("Invalid ID: {e:?}"))?)
        .ok_or_else(|| format!("Object {id} not found"))?;
    let mut transaction =
        crate::operations::OperationTransaction::new(Some("Set object properties".into()));
    for (property, raw) in values {
        let descriptor = get_property_descriptors(object.object_type())
            .into_iter()
            .find(|d| d.name == property)
            .ok_or_else(|| format!("Unknown property '{property}'"))?;
        if !descriptor.writable {
            return Err(format!("Property '{property}' is read-only"));
        }
        transaction.add_operation(
            if matches!(
                descriptor.semantic,
                PropertySemantic::ObjectReference { .. }
            ) {
                Operation::SetObjectReference {
                    object: ObjectTarget::id(id),
                    property,
                    reference: if raw == "null" {
                        None
                    } else {
                        Some(object_target(project, &raw)?)
                    },
                }
            } else {
                let prototype = get_object_property(object, property).map_err(|e| e.to_string())?;
                Operation::SetProperty {
                    object: ObjectTarget::id(id),
                    property,
                    value: parse_property_value(
                        project,
                        object.object_type(),
                        property,
                        &prototype,
                        &raw,
                    )?,
                }
            },
        );
    }
    project
        .execute_transaction(transaction)
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn edit_value(prototype: &Value, inputs: &[String], complete: bool) -> Result<Value, String> {
    if !prototype.is_object() && !prototype.is_array() {
        let [raw] = inputs else {
            return Err("Expected one scalar value".into());
        };
        return parse_scalar(prototype, raw);
    }
    let mut value = prototype.clone();
    let mut fields = std::collections::HashSet::new();
    for input in inputs {
        let (path, raw) = input
            .split_once('=')
            .ok_or_else(|| format!("Expected field=value, got '{input}'"))?;
        if !fields.insert(path) {
            return Err(format!("Field '{path}' was supplied more than once"));
        }
        let replacement = parse_scalar(get_path(&value, path)?, raw)?;
        set_path(&mut value, path, replacement)?;
    }
    if inputs.is_empty() {
        return Err("Supply at least one field=value assignment".into());
    }
    if complete {
        fn leaves(value: &Value, prefix: String, result: &mut Vec<String>) {
            match value {
                Value::Object(object) => {
                    for (key, value) in object {
                        leaves(
                            value,
                            if prefix.is_empty() {
                                key.clone()
                            } else {
                                format!("{prefix}.{key}")
                            },
                            result,
                        );
                    }
                }
                Value::Array(array) => {
                    for (index, value) in array.iter().enumerate() {
                        leaves(value, format!("{prefix}.{index}"), result);
                    }
                }
                _ => result.push(prefix),
            }
        }
        let mut required = Vec::new();
        leaves(prototype, String::new(), &mut required);
        if let Some(missing) = required.iter().find(|p| !fields.contains(p.as_str())) {
            return Err(format!("Missing required field '{missing}'"));
        }
    }
    Ok(value)
}

fn edit_array(
    value: &mut Value,
    property: PropertyKey,
    path: &str,
    edit: ArrayEdit,
) -> Result<(), String> {
    let array = value.as_array_mut().ok_or("Field is not an array")?;
    match edit {
        ArrayEdit::Set(index, assignments) => {
            let slot = array.get_mut(index).ok_or("Array index is out of bounds")?;
            *slot = edit_value(slot, &assignments, false)?;
        }
        ArrayEdit::Append(assignments) => {
            let prototype =
                crate::object_properties::property_array_element_schema_at(property, path)
                    .or_else(|| array.first().cloned())
                    .ok_or_else(|| format!("No element schema for '{property}.{path}'"))?;
            array.push(edit_value(&prototype, &assignments, true)?);
        }
        ArrayEdit::Remove(index) => {
            if index >= array.len() {
                return Err("Array index is out of bounds".into());
            }
            array.remove(index);
        }
        ArrayEdit::Clear => array.clear(),
    }
    Ok(())
}

fn settings(project: &mut EditorProject, values: BTreeMap<String, String>) -> Result<(), String> {
    let mut mask = project.mask_size;
    let mut size = project.soft_key_size();
    let mut version = project.vt_version();
    let mut layout = project.soft_key_layout();
    let mut id_ranges = project.object_id_ranges().clone();
    for (key, value) in values {
        if let Some(object_type) = key.strip_prefix("id_range.") {
            let object_type: ObjectType = serde_json::from_value(json!(object_type))
                .map_err(|_| format!("Unknown object type '{object_type}'"))?;
            let (start, end) = value
                .split_once(':')
                .ok_or_else(|| format!("{key} must be START:END"))?;
            let start: u16 = start
                .parse()
                .map_err(|_| format!("{key} start must be an integer from 0 to 65535"))?;
            let end: u16 = end
                .parse()
                .map_err(|_| format!("{key} end must be an integer from 0 to 65535"))?;
            id_ranges.set_range(object_type, crate::ObjectIdRange { start, end });
            continue;
        }
        match key.as_str() {
            "mask_size" => {
                mask = value
                    .parse()
                    .map_err(|_| "mask_size must be an integer from 1 to 65535")?
            }
            "soft_key_width" => {
                size.0 = value
                    .parse()
                    .map_err(|_| "soft_key_width must be an integer from 1 to 65535")?
            }
            "soft_key_height" => {
                size.1 = value
                    .parse()
                    .map_err(|_| "soft_key_height must be an integer from 1 to 65535")?
            }
            "vt_version" => {
                let raw: u8 = value
                    .parse()
                    .map_err(|_| "vt_version must be between 2 and 6")?;
                if !(2..=6).contains(&raw) {
                    return Err("vt_version must be between 2 and 6".into());
                }
                version = ag_iso_stack::object_pool::vt_version::VtVersion::try_from(raw)
                    .map_err(|_| "Unsupported VT version")?;
            }
            "arrangement" => {
                layout.arrangement = match value.as_str() {
                    "right" | "Right" => crate::soft_key_layout::SoftKeyArrangement::Right,
                    "bottom" | "Bottom" => crate::soft_key_layout::SoftKeyArrangement::Bottom,
                    _ => return Err("arrangement must be right or bottom".into()),
                }
            }
            "physical_key_count" => {
                layout.physical_key_count = value
                    .parse()
                    .map_err(|_| "physical_key_count must be an integer from 0 to 255")?
            }
            "keys_per_line" => {
                layout.keys_per_line = value
                    .parse()
                    .map_err(|_| "keys_per_line must be an integer from 0 to 255")?
            }
            "navigation_key_count" => {
                layout.navigation_key_count = value
                    .parse()
                    .map_err(|_| "navigation_key_count must be an integer from 0 to 255")?
            }
            _ => return Err(format!("Unknown project setting '{key}'")),
        }
    }
    project
        .set_project_configuration_with_id_ranges(mask, size, version, layout, id_ranges)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lexical_values_follow_schema_and_preserve_text() {
        assert_eq!(parse_scalar(&json!(""), "0012").unwrap(), json!("0012"));
        assert_eq!(
            parse_scalar(&json!(""), "{\"x\":1}").unwrap(),
            json!("{\"x\":1}")
        );
        assert_eq!(parse_scalar(&json!(0), "12").unwrap(), json!(12));
        assert!(parse_scalar(&json!(0), "1.5").is_err());
        assert!(parse_scalar(&json!(false), "1").is_err());
        assert!(parse_scalar(&json!({"x":0}), "{\"x\":1}").is_err());
    }

    #[test]
    fn field_updates_preserve_unrelated_content_and_reject_invalid_paths() {
        let original = json!({"enabled":false,"nested":{"value":10,"text":"012"}});
        let result = edit_value(
            &original,
            &["enabled=true".into(), "nested.value=20".into()],
            false,
        )
        .unwrap();
        assert_eq!(
            result,
            json!({"enabled":true,"nested":{"value":20,"text":"012"}})
        );
        assert!(edit_value(&original, &["unknown=1".into()], false).is_err());
        assert!(edit_value(
            &original,
            &["enabled=true".into(), "enabled=false".into()],
            false
        )
        .is_err());
    }

    #[test]
    fn array_insertion_requires_complete_typed_records() {
        let schema =
            crate::object_properties::property_array_element_schema(PropertyKey::Points).unwrap();
        assert_eq!(
            edit_value(&schema, &["x=-12".into(), "y=30".into()], true).unwrap(),
            json!({"x":-12,"y":30})
        );
        assert!(edit_value(&schema, &["x=12".into()], true)
            .unwrap_err()
            .contains("y"));
        assert!(edit_value(&schema, &["x=12".into(), "y=bad".into()], true).is_err());
    }

    #[test]
    fn nested_array_commands_append_and_remove_typed_character_ranges() {
        let mut project = EditorProject::default();
        operation(
            &mut project,
            &Operation::CreateObject {
                object_id: Some(200),
                object_type: ObjectType::ExtendedInputAttributes,
                name: "Characters".into(),
            },
        )
        .unwrap();
        let target = ObjectTarget::id(200);
        structured_property(
            &mut project,
            target.clone(),
            PropertyKey::CodePlanes,
            StructuredEdit::Array(String::new(), ArrayEdit::Append(vec!["number=0".into()])),
        )
        .unwrap();
        structured_property(
            &mut project,
            target.clone(),
            PropertyKey::CodePlanes,
            StructuredEdit::Array(
                "0.character_ranges".into(),
                ArrayEdit::Append(vec![
                    "first_character=65".into(),
                    "last_character=90".into(),
                ]),
            ),
        )
        .unwrap();
        let read = |project: &EditorProject| {
            get_object_property(
                project
                    .get_pool()
                    .object_by_id(ObjectId::new(200).unwrap())
                    .unwrap(),
                PropertyKey::CodePlanes,
            )
            .unwrap()
        };
        assert_eq!(
            read(&project),
            json!([{"number": 0, "character_ranges": [{"first_character":65,"last_character":90}]}])
        );
        structured_property(
            &mut project,
            target.clone(),
            PropertyKey::CodePlanes,
            StructuredEdit::Array(String::new(), ArrayEdit::Append(vec!["number=1".into()])),
        )
        .unwrap();
        assert_eq!(read(&project)[1], json!({"number":1,"character_ranges":[]}));
        structured_property(
            &mut project,
            target,
            PropertyKey::CodePlanes,
            StructuredEdit::Array("0.character_ranges".into(), ArrayEdit::Remove(0)),
        )
        .unwrap();
        assert_eq!(
            read(&project),
            json!([{"number": 0, "character_ranges": []},{"number":1,"character_ranges":[]}])
        );
    }

    #[test]
    fn command_edits_use_project_transactions_and_validate_settings() {
        let mut project = EditorProject::default();
        operation(
            &mut project,
            &Operation::CreateObject {
                object_id: Some(100),
                object_type: ObjectType::Button,
                name: "Start".into(),
            },
        )
        .unwrap();
        set_properties(
            &mut project,
            ObjectTarget::Name {
                name: "Start".into(),
            },
            BTreeMap::from([(PropertyKey::Width, "120".into())]),
        )
        .unwrap();
        let button = project
            .get_pool()
            .object_by_id(ObjectId::new(100).unwrap())
            .unwrap();
        assert_eq!(
            get_object_property(button, PropertyKey::Width).unwrap(),
            json!(120)
        );
        assert!(settings(
            &mut project,
            BTreeMap::from([("mask_size".into(), "0".into())])
        )
        .is_err());
        assert!(settings(
            &mut project,
            BTreeMap::from([("vt_version".into(), "7".into())])
        )
        .is_err());
        assert!(settings(
            &mut project,
            BTreeMap::from([("unknown".into(), "1".into())])
        )
        .is_err());
        assert_eq!(project.mask_size, 500);
    }

    #[test]
    fn execute_batch_checks_without_mutating_and_commits_atomically() {
        let action = EditAction::new(EditActionKind::Operation {
            operation: Operation::CreateObject {
                object_id: Some(100),
                object_type: ObjectType::OutputString,
                name: "Status".into(),
            },
        });
        let mut project = EditorProject::default();

        let checked = execute_batch(
            &mut project,
            json!({"actions": [action.clone()], "check": true}),
        )
        .unwrap();
        assert_eq!(checked["check"], true);
        assert!(project
            .get_pool()
            .object_by_id(ObjectId::new(100).unwrap())
            .is_none());

        let applied =
            execute_batch(&mut project, json!({"actions": [action], "check": false})).unwrap();
        assert_eq!(applied["allocated_ids"], json!([100]));
        assert!(applied["transaction_id"].is_string());
        assert!(project
            .get_pool()
            .object_by_id(ObjectId::new(100).unwrap())
            .is_some());
    }

    #[test]
    fn execute_batch_enforces_action_count_bounds() {
        let error =
            execute_batch(&mut EditorProject::default(), json!({"actions": []})).unwrap_err();
        assert_eq!(error.code, "INVALID_ARGUMENTS");
        let action = EditAction::new(EditActionKind::Settings {
            values: BTreeMap::from([("mask_size".into(), "500".into())]),
        });
        let mut project = EditorProject::default();
        execute_batch(
            &mut project,
            json!({"actions": vec![action.clone(); MAX_EDIT_ACTIONS], "check": true}),
        )
        .unwrap();
        let oversized = execute_batch(
            &mut project,
            json!({"actions": vec![action; MAX_EDIT_ACTIONS + 1]}),
        )
        .unwrap_err();
        assert_eq!(oversized.code, "INVALID_ARGUMENTS");
    }
}
