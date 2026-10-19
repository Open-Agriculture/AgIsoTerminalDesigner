#[cfg(not(target_arch = "wasm32"))]
mod native {
    use ag_iso_stack::object_pool::ObjectType;
    use ag_iso_terminal_designer::cli::protocol::AutomationAction;
    use ag_iso_terminal_designer::cli::session::Session;
    use ag_iso_terminal_designer::{
        cli::editing::{
            EditAction, EditActionKind, ParameterInput, RelationshipEdit, MAX_EDIT_ACTIONS,
        },
        components::{ComponentParameterKind, ComponentParameterTarget},
        object_properties::PropertyKey,
        operations::{ObjectTarget, Operation},
    };
    use clap::{CommandFactory, Parser, Subcommand};
    use serde_json::{json, Value};
    use std::cell::RefCell;
    use std::{
        collections::BTreeMap,
        fs,
        io::{self, Read},
        path::{Path, PathBuf},
        process::ExitCode,
    };

    #[derive(Debug, Parser)]
    #[command(
        name = "aitd",
        about = "Inspect and edit a standalone working project in this shell"
    )]
    struct Cli {
        /// Validate an edit without changing the project.
        #[arg(long, global = true)]
        check: bool,
        /// Print structured JSON instead of readable text.
        #[arg(long, global = true)]
        json: bool,
        #[command(subcommand)]
        command: Command,
    }

    #[derive(Debug, Subcommand, Clone)]
    enum Command {
        /// Show working project status.
        Status,
        /// Describe commands supported by this CLI.
        Capabilities,
        /// Discover retained project backups.
        Backups {
            #[command(subcommand)]
            command: BackupCommand,
        },
        /// Generate a Bash completion script for this command tree.
        Completions {
            #[arg(value_parser = ["bash"])]
            shell: String,
        },
        /// Inspect a mask or container hierarchy.
        Tree {
            root: u16,
            #[arg(long, default_value_t = 6)]
            depth: usize,
        },
        /// Read a design context.
        Context {
            #[arg(long, default_value = "active_mask")]
            scope: String,
            #[arg(long)]
            root: Option<u16>,
            #[arg(long, default_value_t = 3)]
            depth: usize,
            #[arg(long, default_value_t = 50)]
            limit: usize,
            #[arg(long, default_value_t = 0)]
            offset: usize,
            #[arg(long, default_value_t = 0)]
            layout_offset: usize,
            #[arg(long)]
            include_properties: bool,
            #[arg(long, action = clap::ArgAction::Set, default_value_t = true)]
            include_references: bool,
            #[arg(long, action = clap::ArgAction::Set, default_value_t = true)]
            include_selection: bool,
        },
        /// Search project objects.
        Object {
            #[command(subcommand)]
            command: ObjectCommand,
        },
        /// Assign, remap, or reorder object IDs.
        Id {
            #[command(subcommand)]
            command: IdCommand,
        },
        /// Read writable properties and relationship rules.
        Schema {
            object_types: Vec<String>,
            /// Return only the requested properties.
            #[arg(long, value_delimiter = ',')]
            properties: Vec<String>,
        },
        /// Validate the current project.
        Validate,
        /// Render a preview to a PNG file.
        Preview {
            #[arg(long)]
            mask: Option<u16>,
            #[arg(long)]
            output: PathBuf,
            #[arg(long)]
            force: bool,
        },
        /// Save generated IOP, C header, or both.
        Export {
            #[command(subcommand)]
            command: ExportCommand,
        },
        /// Apply edit commands atomically to the working project.
        Run {
            #[arg(long)]
            file: String,
        },
        /// List reusable components.
        Component {
            #[command(subcommand)]
            command: ComponentCommand,
        },
        /// Inspect or edit graphics.
        Graphic {
            #[command(subcommand)]
            command: GraphicsCommand,
        },
        /// Inspect a specific design region.
        Region {
            root: u16,
            #[arg(long,default_value="all",value_parser=["top","bottom","left","right","center","top_left","top_right","bottom_left","bottom_right","all"])]
            region: String,
            #[arg(long)]
            types: Vec<String>,
            #[arg(long, default_value_t = 12)]
            limit: usize,
        },
        /// Resolve colour indexes in the current project.
        Colours { colours: Vec<String> },
        /// Show project settings or edit object ID ranges.
        Project {
            #[command(subcommand)]
            command: ProjectCommand,
        },
        /// Inspect component instances.
        Instance {
            #[command(subcommand)]
            command: InstanceCommand,
        },
        /// Edit a specific parent-child placement.
        Placement {
            #[command(subcommand)]
            command: PlacementCommand,
        },
        /// Set an object's reference property.
        Reference {
            #[command(subcommand)]
            command: ReferenceCommand,
        },
        /// Edit allowed pointer targets.
        Pointer {
            #[command(subcommand)]
            command: PointerCommand,
        },
        /// Edit an object's ordered reference list.
        List {
            #[command(subcommand)]
            command: ListCommand,
        },
        /// Bind or reorder event-to-macro actions.
        Event {
            #[command(subcommand)]
            command: EventCommand,
        },
        /// Add or edit object labels.
        Label {
            #[command(subcommand)]
            command: LabelCommand,
        },
    }
    #[derive(Debug, Subcommand, Clone)]
    enum BackupCommand {
        List {
            #[arg(long)]
            project: Option<PathBuf>,
        },
    }
    #[derive(Debug, Subcommand, Clone)]
    enum ObjectCommand {
        List {
            #[arg(long)]
            query: Option<String>,
            #[arg(long = "type")]
            object_type: Option<String>,
            #[arg(long)]
            parent: Option<u16>,
            #[arg(long, default_value_t = 0)]
            offset: usize,
            #[arg(long, default_value_t = 100)]
            limit: usize,
            /// Read all matching pages.
            #[arg(long)]
            all: bool,
        },
        Get {
            id: String,
            #[arg(long)]
            parents: bool,
            #[arg(long)]
            children: bool,
            #[arg(long)]
            macros: bool,
            /// Include factual style examples from descendants.
            #[arg(long)]
            style: bool,
            #[arg(long, default_value_t = 0)]
            offset: usize,
            #[arg(long, default_value_t = 0)]
            parent_offset: usize,
            #[arg(long, default_value_t = 0)]
            property_offset: usize,
        },
        Create {
            object_type: String,
            #[arg(required_unless_present = "named", conflicts_with = "named")]
            name: Option<String>,
            #[arg(long = "name")]
            named: Option<String>,
            /// Assign an explicit object ID.
            #[arg(long)]
            id: Option<u16>,
        },
        Set {
            id: String,
            #[arg(required = true)]
            assignments: Vec<String>,
        },
        Rename {
            id: String,
            name: String,
        },
        Delete {
            id: String,
        },
        Order {
            ids: Vec<String>,
        },
        Duplicate {
            id: u16,
            #[arg(long)]
            name: Option<String>,
        },
        Field {
            id: String,
            property: String,
            #[arg(required = true)]
            assignments: Vec<String>,
        },
        Array {
            #[command(subcommand)]
            command: ObjectArrayCommand,
        },
        BytesImport {
            id: String,
            property: String,
            file: PathBuf,
        },
        /// Save a byte-valued property without JSON encoding.
        BytesExport {
            id: String,
            property: String,
            #[arg(long)]
            output: PathBuf,
            #[arg(long)]
            force: bool,
        },
    }
    #[derive(Debug, Subcommand, Clone)]
    enum ObjectArrayCommand {
        Set {
            object: String,
            property: String,
            index: u16,
            #[arg(required = true)]
            values: Vec<String>,
        },
        Append {
            object: String,
            property: String,
            #[arg(required = true)]
            values: Vec<String>,
        },
        Remove {
            object: String,
            property: String,
            index: u16,
        },
        Clear {
            object: String,
            property: String,
        },
    }
    #[derive(Debug, Subcommand, Clone)]
    enum ComponentCommand {
        List {
            #[arg(long)]
            query: Option<String>,
            #[arg(long)]
            category: Option<String>,
            #[arg(long, default_value_t = 50)]
            limit: u8,
        },
        Get {
            id: String,
            #[arg(long)]
            nodes: bool,
        },
        Instantiate {
            component_id: String,
            parent: u16,
            x: i32,
            y: i32,
        },
        Move {
            instance_id: String,
            x: i32,
            y: i32,
        },
        Duplicate {
            instance_id: String,
            x: i32,
            y: i32,
        },
        Delete {
            instance_id: String,
        },
        Detach {
            instance_id: String,
        },
        Override {
            instance_id: String,
            parameter: String,
            value: String,
        },
        Reset {
            instance_id: String,
            parameter: String,
        },
        DefinitionRename {
            component_id: String,
            name: String,
        },
        DefinitionDuplicate {
            component_id: String,
        },
        DefinitionDelete {
            component_id: String,
        },
        NodeSet {
            component_id: String,
            node_id: String,
            property: String,
            value: String,
        },
        /// Assign a node reference to an object, node:UUID, or null.
        NodeReference {
            component_id: String,
            node_id: String,
            property: String,
            target: String,
        },
        ParameterRemove {
            component_id: String,
            parameter_id: String,
        },
        Capture {
            root: u16,
            name: String,
        },
        ParameterAdd {
            component_id: String,
            name: String,
            target_node: String,
            property: String,
            kind: String,
            default_value: String,
        },
        ParameterSet {
            component_id: String,
            parameter_id: String,
            name: String,
            target_node: String,
            property: String,
            kind: String,
            default_value: String,
        },
    }
    #[derive(Debug, Subcommand, Clone)]
    enum ProjectCommand {
        /// Load a project into this shell session.
        Load {
            path: PathBuf,
            #[arg(long)]
            discard: bool,
        },
        /// Start a new working project.
        New {
            #[arg(long)]
            discard: bool,
        },
        /// Close this shell session project.
        Close {
            #[arg(long)]
            discard: bool,
        },
        /// Validate and atomically save the working project.
        Save {
            path: PathBuf,
            #[arg(long)]
            force: bool,
            #[arg(long)]
            skip_validation: bool,
        },
        /// Identify the active project, original file path, and current save path.
        Current,
        Settings {
            #[command(subcommand)]
            command: Option<ProjectSettingsCommand>,
        },
        Set {
            #[arg(long)]
            mask_size: Option<u16>,
            #[arg(long)]
            vt_version: Option<u8>,
            #[arg(long)]
            soft_key_width: Option<u16>,
            #[arg(long)]
            soft_key_height: Option<u16>,
        },
        IdRangeSet {
            object_type: String,
            start: u16,
            end: u16,
        },
        IdRangeReset,
    }
    #[derive(Debug, Subcommand, Clone)]
    enum ProjectSettingsCommand {
        Show,
        /// Change settings with typed key=value assignments.
        Set {
            #[arg(required = true)]
            assignments: Vec<String>,
        },
    }
    #[derive(Debug, Subcommand, Clone)]
    enum InstanceCommand {
        List,
        Get { id: String },
    }
    #[derive(Debug, Subcommand, Clone)]
    enum PlacementCommand {
        Add {
            parent: String,
            child: String,
            x: i16,
            y: i16,
            #[arg(long)]
            occurrence: Option<usize>,
        },
        Move {
            parent: String,
            child: String,
            x: i16,
            y: i16,
            #[arg(long)]
            occurrence: Option<usize>,
        },
        Remove {
            parent: String,
            child: String,
            #[arg(long)]
            occurrence: Option<usize>,
        },
        Order {
            parent: String,
            children: Vec<String>,
        },
    }
    #[derive(Debug, Subcommand, Clone)]
    enum GraphicsCommand {
        Get {
            id: String,
        },
        /// Save an available asset or PictureGraphic as a PNG.
        Export {
            id: String,
            #[arg(long)]
            output: PathBuf,
            #[arg(long)]
            force: bool,
        },
        List {
            #[arg(long)]
            query: Option<String>,
            #[arg(long, default_value = "all")]
            status: String,
            #[arg(long, default_value_t = 50)]
            limit: u8,
        },
        Import {
            file: PathBuf,
            #[arg(long)]
            name: Option<String>,
        },
        Assign {
            asset_id: String,
            object: String,
        },
        CreatePicture {
            asset_id: String,
        },
        Rename {
            asset_id: String,
            name: String,
        },
        Delete {
            asset_id: String,
        },
    }
    #[derive(Debug, Subcommand, Clone)]
    enum ExportCommand {
        Iop {
            #[arg(long)]
            output: PathBuf,
            #[arg(long)]
            force: bool,
        },
        Header {
            #[arg(long)]
            output: PathBuf,
            #[arg(long)]
            force: bool,
        },
        Bundle {
            #[arg(long)]
            iop: PathBuf,
            #[arg(long)]
            header: PathBuf,
            #[arg(long)]
            force: bool,
        },
    }

    #[derive(Debug, Subcommand, Clone)]
    enum IdCommand {
        Set {
            object: String,
            id: String,
        },
        Remap {
            #[arg(required = true)]
            mappings: Vec<String>,
        },
        Auto {
            object: String,
        },
        Reassign {
            #[arg(long)]
            include_overrides: bool,
        },
    }
    #[derive(Debug, Subcommand, Clone)]
    enum ReferenceCommand {
        Set {
            object: String,
            property: String,
            target: String,
        },
        Clear {
            object: String,
            property: String,
        },
    }
    #[derive(Debug, Subcommand, Clone)]
    enum PointerCommand {
        Set {
            object: String,
            targets: Vec<String>,
        },
        Clear {
            object: String,
        },
    }
    #[derive(Debug, Subcommand, Clone)]
    enum ListCommand {
        Set {
            object: String,
            targets: Vec<String>,
        },
        Insert {
            object: String,
            index: usize,
            target: String,
        },
        Remove {
            object: String,
            index: usize,
        },
        Clear {
            object: String,
        },
    }
    #[derive(Debug, Subcommand, Clone)]
    enum EventCommand {
        Bind {
            object: String,
            #[arg(required = true)]
            bindings: Vec<String>,
        },
        Set {
            object: String,
            #[arg(required = true)]
            bindings: Vec<String>,
        },
        Unbind {
            object: String,
            event: String,
        },
        Order {
            object: String,
            #[arg(required = true)]
            events: Vec<String>,
        },
        Clear {
            object: String,
        },
    }
    #[derive(Debug, Subcommand, Clone)]
    enum LabelCommand {
        Add {
            object: String,
            #[arg(required = true)]
            fields: Vec<String>,
        },
        Set {
            object: String,
            #[arg(required = true)]
            fields: Vec<String>,
        },
        Remove {
            object: String,
            id: String,
        },
        Clear {
            object: String,
        },
    }

    fn cli_id(value: &str) -> Result<u16, Value> {
        let (digits, radix) = value
            .strip_prefix("0x")
            .map(|v| (v, 16))
            .unwrap_or((value, 10));
        u16::from_str_radix(digits, radix)
            .map_err(|_| error("INVALID_USAGE", format!("'{value}' must be an object ID")))
    }
    /// Convert a Clap-validated command into the shared edit action.
    /// File inputs resolve relative to `base`, which is cwd for shell commands
    /// and the command file's parent for `run --file`.
    fn typed_id<T: serde::de::DeserializeOwned>(value: &str, label: &str) -> Result<T, Value> {
        serde_json::from_value(json!(value))
            .map_err(|e| error("INVALID_USAGE", format!("invalid {label} '{value}': {e}")))
    }
    fn typed_object_type(value: &str) -> Result<ObjectType, Value> {
        typed_id(value, "object type")
    }
    fn typed_property(value: &str) -> Result<PropertyKey, Value> {
        value.parse().map_err(
            |e: ag_iso_terminal_designer::object_properties::UnknownPropertyKey| {
                error("INVALID_USAGE", e.to_string())
            },
        )
    }
    fn typed_property_path(path: &str) -> Result<(PropertyKey, String), Value> {
        let (property, field) = path.split_once('.').unwrap_or((path, ""));
        Ok((typed_property(property)?, field.to_owned()))
    }
    fn typed_target(value: &str) -> ObjectTarget {
        if let Some(name) = value.strip_prefix("name:") {
            ObjectTarget::Name {
                name: name.to_owned(),
            }
        } else if let Ok(id) = cli_id(value) {
            ObjectTarget::Id { id }
        } else {
            ObjectTarget::Name {
                name: value.to_owned(),
            }
        }
    }
    fn typed_parameter_target(value: &str) -> Result<ComponentParameterTarget, Value> {
        if value == "graphic" {
            Ok(ComponentParameterTarget::Graphic)
        } else {
            typed_property(value).map(ComponentParameterTarget::Property)
        }
    }
    fn typed_parameter_kind(value: &str) -> Result<ComponentParameterKind, Value> {
        typed_id(value, "component parameter kind")
    }
    fn edit_action(kind: EditActionKind) -> EditAction {
        EditAction::new(kind)
    }
    fn optional_target(value: &str) -> Option<ObjectTarget> {
        (value != "null").then(|| typed_target(value))
    }
    fn typed_event(
        value: &str,
    ) -> Result<ag_iso_stack::object_pool::object_attributes::Event, Value> {
        use ag_iso_stack::object_pool::object_attributes::Event;
        let event = typed_id(value, "event")?;
        if event == Event::Reserved {
            return Err(error("INVALID_USAGE", "Reserved is not an editable event"));
        }
        Ok(event)
    }
    fn command_edit_actions(
        command: &Command,
        base: &Path,
    ) -> Result<Option<Vec<EditAction>>, Value> {
        let one = |kind| Ok(Some(vec![edit_action(kind)]));
        match command {
            Command::Object { command } => {
                match command {
                    ObjectCommand::Create {
                        object_type,
                        name,
                        named,
                        id,
                    } => one(EditActionKind::Operation {
                        operation: Operation::CreateObject {
                            object_id: *id,
                            object_type: typed_object_type(object_type)?,
                            name: name.as_ref().or(named.as_ref()).unwrap().clone(),
                        },
                    }),
                    ObjectCommand::Set {
                        id,
                        assignments,
                    } => {
                        let mut values = BTreeMap::new();
                        for assignment in assignments {
                            let (property, value) = assignment.split_once('=').ok_or_else(|| {
                                error(
                                    "INVALID_USAGE",
                                    format!("expected property=value, got '{assignment}'"),
                                )
                            })?;
                            if values.insert(typed_property(property)?, value.to_owned()).is_some() {
                                return Err(error("INVALID_USAGE",format!("Repeated property '{property}'")));
                            }
                        }
                        one(EditActionKind::SetProperties {
                            object: typed_target(id),
                            values,
                        })
                    }
                    ObjectCommand::Rename { id, name } => one(EditActionKind::Operation {
                        operation: Operation::RenameObject {
                            object: typed_target(id),
                            name: name.clone(),
                        },
                    }),
                    ObjectCommand::Delete { id } => one(EditActionKind::Relationship {
                        object: Some(typed_target(id)),
                        operation: Operation::DeleteObject { object_id: 0 },
                    }),
                    ObjectCommand::Duplicate { id, name } => one(EditActionKind::ObjectDuplicate {
                        object_id: *id,
                        name: name.clone(),
                    }),
                    ObjectCommand::Order { ids } => one(EditActionKind::Operation {
                        operation: Operation::ReorderObjects {
                            object_ids: ids.iter().map(|id| cli_id(id)).collect::<Result<_, _>>()?,
                        },
                    }),
                    ObjectCommand::Field { id, property, assignments } => {
                        let (property,field)=property.split_once('.').unwrap_or((property,""));
                        one(EditActionKind::ObjectFieldSet {object:typed_target(id),property:typed_property(property)?,field:field.to_owned(),values:assignments.clone()})
                    },
                    ObjectCommand::Array { command } => {
                        let (object,path)=match command {
                            ObjectArrayCommand::Set {object,property,..} | ObjectArrayCommand::Append {object,property,..} |
                            ObjectArrayCommand::Remove {object,property,..} | ObjectArrayCommand::Clear {object,property} => (object,property),
                        };
                        let object=typed_target(object);
                        let (property,field)=typed_property_path(path)?;
                        one(match command {
                            ObjectArrayCommand::Set {index,values,..}=>EditActionKind::ObjectArraySet {object,property,field,index:usize::from(*index),values:values.clone()},
                            ObjectArrayCommand::Append {values,..}=>EditActionKind::ObjectArrayAppend {object,property,field,values:values.clone()},
                            ObjectArrayCommand::Remove {index,..}=>EditActionKind::ObjectArrayRemove {object,property,field,index:usize::from(*index)},
                            ObjectArrayCommand::Clear {..}=>EditActionKind::ObjectArrayClear {object,property,field},
                        })
                    },
                    ObjectCommand::BytesImport { id, property, file } => {
                        let bytes = fs::read(base.join(file))
                            .map_err(|e| error("INPUT_FAILED", e.to_string()))?;
                        one(EditActionKind::ObjectBytesSet {
                            object: typed_target(id),
                            property: typed_property(property)?,
                            bytes,
                        })
                    }
                    _ => Ok(None),
                }
            }
            Command::Id { command } => match command {
                IdCommand::Set { object, id } => one(EditActionKind::Operation { operation: Operation::SetObjectIdAssignment { object_id: cli_id(object)?, new_id: cli_id(id)?, overridden: true } }),
                IdCommand::Auto { object } => one(EditActionKind::ObjectIdAuto { object_id: cli_id(object)? }),
                IdCommand::Reassign { include_overrides } => one(EditActionKind::ObjectIdReassign { include_overrides: *include_overrides, mappings: None }),
                IdCommand::Remap { mappings } => one(EditActionKind::Operation { operation: Operation::RemapObjectIds {
                    mappings: mappings.iter().map(|pair| {
                        let (old,new)=pair.split_once(':').ok_or_else(|| error("INVALID_USAGE", "expected old:new ID pair"))?;
                        Ok((cli_id(old)?,cli_id(new)?))
                    }).collect::<Result<_,Value>>()?, target_types: Vec::new(),
                } }),
            },
            Command::Project {
                command:
                    ProjectCommand::Set {
                        mask_size,
                        vt_version,
                        soft_key_width,
                        soft_key_height,
                    },
            } => {
                let values = [
                    ("mask_size", mask_size.map(|v| v.to_string())),
                    ("vt_version", vt_version.map(|v| v.to_string())),
                    ("soft_key_width", soft_key_width.map(|v| v.to_string())),
                    ("soft_key_height", soft_key_height.map(|v| v.to_string())),
                ]
                .into_iter()
                .filter_map(|(key, value)| value.map(|value| (key.to_owned(), value)))
                .collect();
                one(EditActionKind::Settings { values })
            }
            Command::Project {
                command:
                    ProjectCommand::Settings {
                        command: Some(ProjectSettingsCommand::Set { assignments }),
                    },
            } => {
                let mut values = BTreeMap::new();
                for assignment in assignments {
                    let (key, value) = assignment.split_once('=').ok_or_else(|| {
                        error(
                            "INVALID_USAGE",
                            format!("expected key=value, got '{assignment}'"),
                        )
                    })?;
                    if values.insert(key.to_owned(), value.to_owned()).is_some() {
                        return Err(error("INVALID_USAGE",format!("Repeated setting '{key}'")));
                    }
                }
                one(EditActionKind::Settings { values })
            }
            Command::Project {
                command:
                    ProjectCommand::IdRangeSet {
                        object_type,
                        start,
                        end,
                    },
            } => one(EditActionKind::IdRangeSet {
                object_type: typed_object_type(object_type)?,
                start: *start,
                end: *end,
            }),
            Command::Project {
                command: ProjectCommand::IdRangeReset,
            } => one(EditActionKind::IdRangeReset),
            Command::Placement { command } => match command {
                PlacementCommand::Add {
                    parent,
                    child,
                    x,
                    y,
                    occurrence,
                } => {
                    if occurrence.is_some() {
                        return Err(error(
                            "INVALID_USAGE",
                            "placement add does not accept --occurrence",
                        ));
                    }
                    one(EditActionKind::Operation {
                        operation: Operation::AddChild {
                            parent: typed_target(parent),
                            child: typed_target(child),
                            x: *x,
                            y: *y,
                        },
                    })
                }
                PlacementCommand::Move {
                    parent,
                    child,
                    x,
                    y,
                    occurrence,
                } => one(EditActionKind::RelationshipEdit {
                    edit: RelationshipEdit::PlacementMove {
                        parent: typed_target(parent),
                        child: typed_target(child),
                        x: *x,
                        y: *y,
                        occurrence: *occurrence,
                    },
                }),
                PlacementCommand::Remove {
                    parent,
                    child,
                    occurrence,
                } => one(EditActionKind::RelationshipEdit {
                    edit: RelationshipEdit::PlacementRemove {
                        parent: typed_target(parent),
                        child: typed_target(child),
                        occurrence: *occurrence,
                    },
                }),
                PlacementCommand::Order { parent, children } => {
                    if children.is_empty() {
                        return Err(error(
                            "INVALID_USAGE",
                            "placement order requires every child occurrence",
                        ));
                    }
                    one(EditActionKind::RelationshipEdit {
                        edit: RelationshipEdit::PlacementOrder {
                            parent: typed_target(parent),
                            children: children.iter().map(|child| typed_target(child)).collect(),
                        },
                    })
                }
            },
            Command::Reference { command } => {
                let (object,property,target)=match command {
                    ReferenceCommand::Set { object,property,target } => (object,property,optional_target(target)),
                    ReferenceCommand::Clear { object,property } => (object,property,None),
                };
                one(EditActionKind::Operation { operation: Operation::SetObjectReference { object:typed_target(object),property:typed_property(property)?,reference:target } })
            }
            Command::Pointer { command } => {
                let (object,targets)=match command {
                    PointerCommand::Set { object,targets } => (object,targets.as_slice()),
                    PointerCommand::Clear { object } => (object,&[][..]),
                };
                one(EditActionKind::Operation { operation: Operation::SetObjectPointerAllowedReferences { object:typed_target(object), references:targets.iter().map(|s| typed_target(s)).collect() } })
            }
            Command::List { command } => one(EditActionKind::RelationshipEdit { edit:match command {
                ListCommand::Set { object,targets } => RelationshipEdit::ListSet { object:typed_target(object), required:None,objects:targets.iter().map(|s|optional_target(s)).collect() },
                ListCommand::Clear { object } => RelationshipEdit::ListSet { object:typed_target(object), required:None,objects:Vec::new() },
                ListCommand::Insert { object,index,target } => RelationshipEdit::ListInsert { object:typed_target(object),index:*index,target:optional_target(target) },
                ListCommand::Remove { object,index } => RelationshipEdit::ListRemove { object:typed_target(object),index:*index },
            } }),
            Command::Event { command } => one(EditActionKind::RelationshipEdit { edit:match command {
                EventCommand::Bind { object,bindings } | EventCommand::Set { object,bindings } => RelationshipEdit::MacroBindings {
                    object:typed_target(object),replace:matches!(command,EventCommand::Set {..}),
                    bindings:bindings.iter().map(|binding| {
                        let (event,target)=binding.split_once('=').ok_or_else(||error("INVALID_USAGE","expected EVENT=TARGET"))?;
                        Ok(ag_iso_terminal_designer::MacroReferenceTarget { event_id:typed_event(event)?,macro_object:typed_target(target) })
                    }).collect::<Result<_,Value>>()?,
                },
                EventCommand::Clear { object } => RelationshipEdit::MacroBindings { object:typed_target(object),replace:true,bindings:Vec::new() },
                EventCommand::Unbind { object,event } => RelationshipEdit::MacroUnbind { object:typed_target(object),event:typed_event(event)? },
                EventCommand::Order { object,events } => RelationshipEdit::MacroOrder { object:typed_target(object),events:events.iter().map(|e|typed_event(e)).collect::<Result<_,_>>()? },
            } }),
            Command::Label { command } => one(EditActionKind::RelationshipEdit { edit:match command {
                LabelCommand::Clear { object } => RelationshipEdit::LabelClear { object:typed_target(object) },
                LabelCommand::Remove { object,id } => RelationshipEdit::LabelRemove { object:typed_target(object),id:typed_target(id) },
                LabelCommand::Add { object,fields } | LabelCommand::Set { object,fields } => {
                    let mut values=BTreeMap::new();
                    for field in fields {
                        let (key,value)=field.split_once('=').ok_or_else(||error("INVALID_USAGE","expected FIELD=VALUE"))?;
                        if !["id","string","graphic","font"].contains(&key) || values.insert(key,value).is_some() { return Err(error("INVALID_USAGE",format!("Unknown or repeated label field '{key}'"))); }
                    }
                    RelationshipEdit::LabelSet { object:typed_target(object),replace:matches!(command,LabelCommand::Set {..}),
                        id:typed_target(values.get("id").ok_or_else(||error("INVALID_USAGE","label requires id=TARGET"))?),
                        string:values.get("string").and_then(|s|optional_target(s)),graphic:values.get("graphic").and_then(|s|optional_target(s)),
                        font:values.get("font").ok_or_else(||error("INVALID_USAGE","label requires font=NUMBER"))?.parse::<u8>().map_err(|_|error("INVALID_USAGE","font must be 0..255"))?,
                    }
                }
            } }),
            Command::Component { command } => match command {
                ComponentCommand::Instantiate {
                    component_id,
                    parent,
                    x,
                    y,
                } => one(EditActionKind::Component {
                    action:
                        ag_iso_terminal_designer::components::ComponentIntent::InstantiateComponent {
                            instance_id: None,
                            component_id: typed_id(component_id, "component ID")?,
                            parent: *parent,
                            x: *x,
                            y: *y,
                            overrides: BTreeMap::new(),
                        },
                }),
                ComponentCommand::Move { instance_id, x, y } => one(EditActionKind::ComponentMove {
                    instance_id: typed_id(instance_id, "component instance ID")?,
                    x: *x,
                    y: *y,
                }),
                ComponentCommand::Duplicate { instance_id, x, y } => {
                    one(EditActionKind::ComponentDuplicate {
                        instance_id: typed_id(instance_id, "component instance ID")?,
                        x: *x,
                        y: *y,
                    })
                }
                ComponentCommand::Delete { instance_id } => one(EditActionKind::ComponentDelete {
                    instance_id: typed_id(instance_id, "component instance ID")?,
                }),
                ComponentCommand::Detach { instance_id } => one(EditActionKind::ComponentDetach {
                    instance_id: typed_id(instance_id, "component instance ID")?,
                }),
                ComponentCommand::Override {
                    instance_id,
                    parameter,
                    value,
                } => one(EditActionKind::ComponentOverride {
                    instance_id: typed_id(instance_id, "component instance ID")?,
                    parameter: parameter.clone(),
                    value: value.clone(),
                }),
                ComponentCommand::Reset {
                    instance_id,
                    parameter,
                } => one(EditActionKind::ComponentReset {
                    instance_id: typed_id(instance_id, "component instance ID")?,
                    parameter: parameter.clone(),
                }),
                ComponentCommand::DefinitionRename { component_id, name } => {
                    one(EditActionKind::ComponentDefinitionRename {
                        component_id: typed_id(component_id, "component ID")?,
                        name: name.clone(),
                    })
                }
                ComponentCommand::DefinitionDuplicate { component_id } => {
                    one(EditActionKind::ComponentDefinitionDuplicate {
                        component_id: typed_id(component_id, "component ID")?,
                    })
                }
                ComponentCommand::DefinitionDelete { component_id } => {
                    one(EditActionKind::ComponentDefinitionDelete {
                        component_id: typed_id(component_id, "component ID")?,
                    })
                }
                ComponentCommand::NodeSet {
                    component_id,
                    node_id,
                    property,
                    value,
                } => one(EditActionKind::ComponentNodeSet {
                    component_id: typed_id(component_id, "component ID")?,
                    node_id: typed_id(node_id, "component node ID")?,
                    property: typed_property(property)?,
                    value: value.clone(),
                }),
                ComponentCommand::NodeReference {
                    component_id,
                    node_id,
                    property,
                    target,
                } => one(EditActionKind::ComponentNodeReference {
                    component_id: typed_id(component_id, "component ID")?,
                    node_id: typed_id(node_id, "component node ID")?,
                    property: typed_property(property)?,
                    target: target.clone(),
                }),
                ComponentCommand::ParameterRemove {
                    component_id,
                    parameter_id,
                } => one(EditActionKind::ComponentParameterRemove {
                    component_id: typed_id(component_id, "component ID")?,
                    parameter_id: typed_id(parameter_id, "component parameter ID")?,
                }),
                ComponentCommand::Capture { root, name } => one(EditActionKind::ComponentCapture {
                    root: *root,
                    name: name.clone(),
                    keep_external_references: false,
                }),
                ComponentCommand::ParameterAdd {
                    component_id,
                    name,
                    target_node,
                    property,
                    kind,
                    default_value,
                } => one(EditActionKind::ComponentParameterAdd {
                    component_id: typed_id(component_id, "component ID")?,
                    parameter: ParameterInput {
                        id: None,
                        name: name.clone(),
                        description: None,
                        target_node: typed_id(target_node, "component node ID")?,
                        property: typed_parameter_target(property)?,
                        kind: typed_parameter_kind(kind)?,
                        default_value: Some(default_value.clone()),
                    },
                }),
                ComponentCommand::ParameterSet {
                    component_id,
                    parameter_id,
                    name,
                    target_node,
                    property,
                    kind,
                    default_value,
                } => one(EditActionKind::ComponentParameterSet {
                    component_id: typed_id(component_id, "component ID")?,
                    parameter: ParameterInput {
                        id: Some(typed_id(parameter_id, "component parameter ID")?),
                        name: name.clone(),
                        description: None,
                        target_node: typed_id(target_node, "component node ID")?,
                        property: typed_parameter_target(property)?,
                        kind: typed_parameter_kind(kind)?,
                        default_value: Some(default_value.clone()),
                    },
                }),
                _ => Ok(None),
            },
            Command::Graphic { command } => match command {
                GraphicsCommand::Import { file, name } => {
                    let path = base.join(file);
                    let bytes = fs::read(&path).map_err(|e| error("INPUT_FAILED", e.to_string()))?;
                    one(EditActionKind::GraphicImport {
                        filename: path
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("graphic")
                            .to_owned(),
                        bytes,
                        name: name.clone(),
                    })
                }
                GraphicsCommand::Assign { asset_id, object } => one(EditActionKind::GraphicAssign {
                    asset_id: typed_id(asset_id, "graphic asset ID")?,
                    object: cli_id(object)?,
                }),
                GraphicsCommand::CreatePicture { asset_id } => {
                    one(EditActionKind::GraphicCreatePicture {
                        asset_id: typed_id(asset_id, "graphic asset ID")?,
                    })
                }
                GraphicsCommand::Rename { asset_id, name } => one(EditActionKind::GraphicRename {
                    asset_id: typed_id(asset_id, "graphic asset ID")?,
                    name: name.clone(),
                }),
                GraphicsCommand::Delete { asset_id } => one(EditActionKind::GraphicDelete {
                    asset_id: typed_id(asset_id, "graphic asset ID")?,
                }),
                _ => Ok(None),
            },
            _ => Ok(None),
        }
    }

    fn error(code: &str, message: impl Into<String>) -> Value {
        json!({"error":{"code":code,"message":message.into()}})
    }

    /// Holds the session lock for the entire invocation, including outputs.
    struct Client {
        session: RefCell<Session>,
        check: bool,
    }
    impl Client {
        fn request(&self, action: AutomationAction, args: Value) -> Result<Value, Value> {
            let session = self.session.borrow();
            let project = session
                .project()
                .map_err(|e| error("PROJECT_NOT_OPEN", e))?;
            let result =
                ag_iso_terminal_designer::cli::automation_service::execute(project, action, args)
                    .map_err(|e| error(e.code, e.message))?;
            Ok(json!({"success":true,"result":result}))
        }
        fn edit(&self, actions: Vec<EditAction>) -> Result<Value, Value> {
            let mut session = self.session.borrow_mut();
            let result = ag_iso_terminal_designer::cli::editing::execute_batch(
                session
                    .project_mut()
                    .map_err(|e| error("PROJECT_NOT_OPEN", e))?,
                json!({"actions":actions,"check":self.check}),
            )
            .map_err(|e| error(e.code, e.message))?;
            if !self.check {
                session.persist().map_err(|e| error("SESSION_FAILED", e))?;
            }
            Ok(json!({"success":true,"result":result}))
        }
        fn output(
            &self,
            action: AutomationAction,
            args: Value,
            field: &str,
            output: &Path,
            force: bool,
        ) -> Result<Value, Value> {
            let mut response = self.request(action, args)?;
            let bytes = decode_output(&response["result"], field)?;
            write_output(output, &bytes, force)?;
            response["result"] = json!({"output":output,"bytes":bytes.len()});
            Ok(response)
        }
    }

    fn execute(cli: Cli) -> Result<Value, Value> {
        use AutomationAction as A;
        if cli.check
            && matches!(
                cli.command,
                Command::Capabilities | Command::Completions { .. } | Command::Backups { .. }
            )
        {
            return Err(error(
                "INVALID_USAGE",
                "--check is only supported by edit commands and run",
            ));
        }
        match &cli.command {
            Command::Capabilities => {
                return Ok(json!({"success":true,"result":{
                    "cli_api_version":4,"json_input":false,"atomic_command_files":true,"max_edit_actions":MAX_EDIT_ACTIONS,
                    "commands":Cli::command().get_subcommands().map(|c|json!({"name":c.get_name(),"description":c.get_about().map(ToString::to_string),"subcommands":c.get_subcommands().map(|s|s.get_name()).collect::<Vec<_>>()})).collect::<Vec<_>>()
                }}))
            }
            Command::Completions { shell } => {
                return Ok(
                    json!({"success":true,"result":{"shell":shell,"script":bash_completions()}}),
                )
            }
            Command::Backups {
                command: BackupCommand::List { project },
            } => {
                return Session::backups_list(project.as_deref())
                    .map(|backups| json!({"success":true,"result":{"backups":backups}}))
                    .map_err(|e| error("BACKUP_FAILED", e));
            }
            _ => {}
        }
        let mut session = Session::open().map_err(|e| error("SESSION_FAILED", e))?;
        if let Command::Project { command } = &cli.command {
            let lifecycle = matches!(
                command,
                ProjectCommand::Load { .. }
                    | ProjectCommand::New { .. }
                    | ProjectCommand::Close { .. }
                    | ProjectCommand::Save { .. }
            );
            if lifecycle {
                if cli.check {
                    return Err(error(
                        "INVALID_USAGE",
                        "--check is only supported by edit commands and run",
                    ));
                }
                let result = match command {
                    ProjectCommand::Load { path, discard } => session
                        .load(path, *discard)
                        .map(|_| json!(session.status()))
                        .map_err(|e| error("PROJECT_FAILED", e)),
                    ProjectCommand::New { discard } => session
                        .new_project(*discard)
                        .map(|_| json!(session.status()))
                        .map_err(|e| error("PROJECT_FAILED", e)),
                    ProjectCommand::Close { discard } => session
                        .close(*discard)
                        .map(|_| json!(session.status()))
                        .map_err(|e| error("PROJECT_FAILED", e)),
                    ProjectCommand::Save {
                        path,
                        force,
                        skip_validation,
                    } => session
                        .save(path, *force, *skip_validation, |project| {
                            let validation =
                                ag_iso_terminal_designer::cli::automation_service::execute(
                                    project,
                                    A::ValidateDesign,
                                    json!({}),
                                )
                                .map_err(|e| e.message)?;
                            if validation["valid"] == true {
                                Ok(())
                            } else {
                                Err(format!(
                                    "Project validation failed: {}",
                                    validation["diagnostics"]
                                ))
                            }
                        })
                        .map(|saved| json!(saved))
                        .map_err(|e| error(e.code, e.message)),
                    _ => unreachable!(),
                }?;
                return Ok(json!({"success":true,"result":result}));
            }
        }
        let status = json!({"success":true,"result":session.status()});
        let client = Client {
            session: RefCell::new(session),
            check: cli.check,
        };
        let base = std::env::current_dir().map_err(|e| error("INPUT_FAILED", e.to_string()))?;
        if let Some(actions) = command_edit_actions(&cli.command, &base)? {
            return client.edit(actions);
        }
        if cli.check && !matches!(cli.command, Command::Run { .. }) {
            return Err(error(
                "INVALID_USAGE",
                "--check is only supported by edit commands and run",
            ));
        }
        match cli.command {
            Command::Status | Command::Project {command:ProjectCommand::Current} => Ok(status),
            Command::Tree {root,depth} => client.request(A::GetDesignContext,json!({"root_id":root,"depth":depth,"max_objects":200})),
            Command::Context {scope,root,depth,limit,offset,layout_offset,include_properties,include_references,include_selection} => client.request(A::GetDesignContext,json!({"scope":scope,"root_id":root,"depth":depth,"max_objects":limit,"offset":offset,"layout_offset":layout_offset,"include_properties":include_properties,"include_references":include_references,"include_selection":include_selection})),
            Command::Schema {object_types,properties} => {
                if object_types.is_empty() || object_types == ["types"] {client.request(A::GetObjectTypes,json!({}))}
                else {client.request(A::GetObjectSchema,json!({"object_types":object_types,"properties":properties}))}
            }
            Command::Object {command} => match command {
                ObjectCommand::List {query,object_type,parent,offset,limit,all} => {
                    let mut args=json!({"query":query,"type":object_type,"parent_id":parent,"offset":offset,"limit":limit});
                    let mut response=client.request(A::ListObjects,args.clone())?;
                    let mut objects=response["result"]["objects"].as_array().ok_or_else(||error("AUTOMATION_FAILED","Object list missing"))?.clone();
                    while all && response["result"]["has_more"].as_bool()==Some(true) {
                        args["offset"]=json!(offset+objects.len());
                        response=client.request(A::ListObjects,args.clone())?;
                        let page=response["result"]["objects"].as_array().ok_or_else(||error("AUTOMATION_FAILED","Object list missing"))?;
                        if page.is_empty() { return Err(error("AUTOMATION_FAILED","Object pagination did not advance")); }
                        objects.extend(page.iter().cloned());
                    }
                    response["result"]["objects"]=json!(objects); response["result"]["offset"]=json!(offset);
                    Ok(response)
                }
                ObjectCommand::Get {id,parents,children,macros,style,offset,parent_offset,property_offset} => client.request(A::GetObjects,json!({"targets":[typed_target(&id)],"include_parents":parents,"include_children":children,"include_macros":macros,"include_nearby_style":style,"detail_offset":offset,"parent_offset":parent_offset,"property_offset":property_offset})),
                ObjectCommand::BytesExport {id,property,output,force} => client.output(A::ExportObjectBytes,json!({"object":typed_target(&id),"property":property}),"bytes_base64",&output,force),
                _=>unreachable!("edit command converted above"),
            },
            Command::Component {command} => match command {
                ComponentCommand::List {query,category,limit}=>client.request(A::ListComponents,json!({"query":query,"category":category,"limit":limit})),
                ComponentCommand::Get {id,nodes}=>client.request(A::GetComponent,json!({"component_id":id,"include_nodes":nodes})),
                _=>unreachable!("edit command converted above"),
            },
            Command::Graphic {command}=>match command {
                GraphicsCommand::Get {id}=>client.request(A::GetGraphic,json!({"id":id})),
                GraphicsCommand::List {query,status,limit}=>client.request(A::ListGraphics,json!({"query":query,"status":status,"limit":limit})),
                GraphicsCommand::Export {id,output,force}=>client.output(A::GetGraphic,json!({"id":id,"export":true}),"/graphic/png_base64",&output,force),
                _=>unreachable!("edit command converted above"),
            },
            Command::Validate=> {
                let mut response = client.request(A::ValidateDesign,json!({}))?;
                if response["result"]["valid"] == false {
                    response["success"] = json!(false);
                    response["error"] = json!({"code":"VALIDATION_FAILED","message":"Project validation failed"});
                    Err(response)
                } else { Ok(response) }
            },
            Command::Preview {mask,output,force}=>client.output(A::PreviewDesign,json!({"mask_id":mask}),"png_base64",&output,force),
            Command::Export {command}=>export(&client,command),
            Command::Run {file}=>run_file(&client,&file),
            Command::Region {root,region,types,limit}=>client.request(A::InspectDesignRegion,json!({"root_id":root,"region":region,"types":types,"max_results":limit})),
            Command::Colours {colours}=>client.request(A::ResolveColours,json!({"colours":colours})),
            Command::Project {command:ProjectCommand::Settings {command:None|Some(ProjectSettingsCommand::Show)}}=>client.request(A::ProjectSettings,json!({})),
            Command::Instance {command}=>client.request(A::GetComponentInstances,match command {InstanceCommand::List=>json!({}),InstanceCommand::Get {id}=>json!({"instance_id":id})}),
            _=>unreachable!("local and edit commands handled above"),
        }
    }

    fn decode_output(result: &Value, field: &str) -> Result<Vec<u8>, Value> {
        let encoded = (if field.starts_with('/') {
            result.pointer(field)
        } else {
            result.get(field)
        })
        .and_then(Value::as_str)
        .ok_or_else(|| {
            error(
                "EXPORT_RESPONSE_INVALID",
                format!("Output response omitted {field}"),
            )
        })?;
        base64::Engine::decode(&base64::engine::general_purpose::STANDARD, encoded)
            .map_err(|e| error("EXPORT_RESPONSE_INVALID", e.to_string()))
    }
    fn write_output(path: &Path, bytes: &[u8], force: bool) -> Result<(), Value> {
        if fs::symlink_metadata(path).is_ok() && !force {
            return Err(error(
                "OUTPUT_EXISTS",
                format!("{} exists; pass --force to replace it.", path.display()),
            ));
        }
        let staged = stage_output(path, bytes)?;
        publish_outputs(std::slice::from_ref(&staged), force)
    }
    struct StagedOutput {
        temporary: PathBuf,
        path: PathBuf,
        bytes: usize,
    }
    impl Drop for StagedOutput {
        fn drop(&mut self) {
            let _ = fs::remove_file(&self.temporary);
        }
    }
    fn stage_output(path: &Path, bytes: &[u8]) -> Result<StagedOutput, Value> {
        if path.is_dir() {
            return Err(error(
                "OUTPUT_FAILED",
                format!("{} is a directory.", path.display()),
            ));
        }
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent).map_err(|e| error("OUTPUT_FAILED", e.to_string()))?;
        }
        let tmp = temporary_output_path(path);
        let staged = StagedOutput {
            temporary: tmp,
            path: path.to_owned(),
            bytes: bytes.len(),
        };
        fs::write(&staged.temporary, bytes).map_err(|e| error("OUTPUT_FAILED", e.to_string()))?;
        Ok(staged)
    }
    fn publish_outputs(staged: &[StagedOutput], force: bool) -> Result<(), Value> {
        let mut destinations = std::collections::HashSet::new();
        for output in staged {
            let parent = output
                .path
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
            let destination = fs::canonicalize(parent)
                .map_err(|e| error("OUTPUT_FAILED", e.to_string()))?
                .join(
                    output
                        .path
                        .file_name()
                        .ok_or_else(|| error("OUTPUT_FAILED", "Output must name a file"))?,
                );
            if !destinations.insert(destination) {
                return Err(error("INVALID_USAGE", "Outputs must use different paths"));
            }
        }
        for output in staged {
            let result = if force {
                fs::rename(&output.temporary, &output.path)
            } else {
                fs::hard_link(&output.temporary, &output.path)
            };
            result.map_err(|e| {
                let code = if e.kind() == io::ErrorKind::AlreadyExists {
                    "OUTPUT_EXISTS"
                } else {
                    "OUTPUT_FAILED"
                };
                error(code, format!("{}: {e}", output.path.display()))
            })?;
        }
        Ok(())
    }
    fn temporary_output_path(path: &Path) -> PathBuf {
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("output");
        path.with_file_name(format!(
            ".{name}.aitd-{}-{}.tmp",
            std::process::id(),
            uuid::Uuid::new_v4()
        ))
    }
    fn export(client: &Client, command: ExportCommand) -> Result<Value, Value> {
        let (format, outputs, force) = match command {
            ExportCommand::Iop { output, force } => ("iop", vec![("iop", output)], force),
            ExportCommand::Header { output, force } => ("header", vec![("header", output)], force),
            ExportCommand::Bundle { iop, header, force } => {
                ("bundle", vec![("iop", iop), ("header", header)], force)
            }
        };
        if outputs.len() == 2 && outputs[0].1 == outputs[1].1 {
            return Err(error(
                "INVALID_USAGE",
                "IOP and header outputs must use different paths",
            ));
        }
        for (_, path) in &outputs {
            if fs::symlink_metadata(path).is_ok() && !force {
                return Err(error(
                    "OUTPUT_EXISTS",
                    format!("{} exists; pass --force to replace it", path.display()),
                ));
            }
        }
        let session = client.session.borrow();
        let project = session
            .project()
            .map_err(|e| error("PROJECT_NOT_OPEN", e))?;
        let artifacts = ag_iso_terminal_designer::cli::export::artifacts(project, format)
            .map_err(|e| error("VALIDATION_FAILED", e.to_string()))?;
        let staged = outputs
            .into_iter()
            .map(|(key, path)| {
                let bytes = match key {
                    "iop" => artifacts.iop.as_deref(),
                    "header" => artifacts.header.as_deref(),
                    _ => unreachable!(),
                }
                .ok_or_else(|| error("EXPORT_FAILED", "Requested artifact is missing"))?;
                stage_output(&path, bytes)
            })
            .collect::<Result<Vec<_>, Value>>()?;
        publish_outputs(&staged, force)?;
        Ok(json!({"success":true,"result":{
            "outputs":staged.iter().map(|o|json!({"path":o.path,"bytes":o.bytes})).collect::<Vec<_>>(),
            "vt_version":artifacts.vt_version
        }}))
    }

    /// Shell-style quoting without expansion or command execution.
    fn split_line(line: &str) -> Result<Vec<String>, String> {
        let mut words = Vec::new();
        let mut word = String::new();
        let mut quote = None;
        let mut started = false;
        let mut chars = line.chars().peekable();
        while let Some(c) = chars.next() {
            if quote == Some('\'') {
                if c == '\'' {
                    quote = None;
                } else {
                    word.push(c);
                }
            } else if c == '\\' {
                let next = chars.next().ok_or("trailing escape")?;
                if quote == Some('"') && !matches!(next, '"' | '\\' | '$' | '`') {
                    word.push('\\');
                }
                word.push(next);
                started = true;
            } else if let Some(q) = quote {
                if c == q {
                    quote = None;
                } else {
                    word.push(c);
                }
            } else if c == '\'' || c == '"' {
                quote = Some(c);
                started = true;
            } else if c.is_whitespace() {
                if started {
                    words.push(std::mem::take(&mut word));
                    started = false;
                }
            } else {
                word.push(c);
                started = true;
            }
        }
        if quote.is_some() {
            return Err("unclosed quote".into());
        }
        if started {
            words.push(word);
        }
        Ok(words)
    }

    fn continues(line: &str) -> bool {
        let mut quote = None;
        let mut escape = false;
        for c in line.chars() {
            if escape {
                escape = false;
            } else if quote == Some('\'') {
                if c == '\'' {
                    quote = None;
                }
            } else if c == '\\' {
                escape = true;
            } else if let Some(q) = quote {
                if c == q {
                    quote = None;
                }
            } else if c == '\'' || c == '"' {
                quote = Some(c);
            }
        }
        escape
    }

    fn run_file(client: &Client, file: &str) -> Result<Value, Value> {
        let (contents, base) = if file == "-" {
            let mut contents = String::new();
            io::stdin()
                .read_to_string(&mut contents)
                .map_err(|e| error("INPUT_FAILED", e.to_string()))?;
            (contents, std::env::current_dir().unwrap_or_default())
        } else {
            let path = PathBuf::from(file);
            (
                fs::read_to_string(&path).map_err(|e| error("INPUT_FAILED", e.to_string()))?,
                path.parent().unwrap_or(Path::new(".")).to_path_buf(),
            )
        };
        let mut actions = Vec::new();
        let mut logical_lines = Vec::new();
        let mut pending = String::new();
        let mut start_line = 1;
        for (index, line) in contents.lines().enumerate() {
            if pending.is_empty() {
                start_line = index + 1;
            }
            pending.push_str(line);
            if continues(&pending) {
                pending.pop();
                pending.push(' ');
            } else {
                logical_lines.push((start_line, std::mem::take(&mut pending)));
            }
        }
        if !pending.is_empty() {
            return Err(error(
                "SCRIPT_PARSE_ERROR",
                format!("line {start_line}: trailing continuation"),
            ));
        }
        for (line_number, line) in logical_lines {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let tokens = split_line(line)
                .map_err(|e| error("SCRIPT_PARSE_ERROR", format!("line {line_number}: {e}")))?;
            let cli = Cli::try_parse_from(
                std::iter::once("aitd").chain(tokens.iter().map(String::as_str)),
            )
            .map_err(|e| error("SCRIPT_PARSE_ERROR", format!("line {line_number}: {e}")))?;
            if cli.json || cli.check {
                return Err(error("SCRIPT_COMMAND_UNSUPPORTED",format!("line {line_number}: select output format and --check on run, not inside the batch")));
            }
            let line_actions = command_edit_actions(&cli.command, &base)
                .map_err(|mut e| {
                    if let Some(message) = e.pointer_mut("/error/message") {
                        if let Some(text) = message.as_str().map(str::to_owned) {
                            *message = json!(format!("line {line_number}: {text}"));
                        }
                    }
                    e
                })?
                .ok_or_else(|| error(
                    "SCRIPT_COMMAND_UNSUPPORTED",
                    format!("line {line_number}: output or inspection commands cannot run in a batch"),
                ))?;
            actions.extend(line_actions.into_iter().map(|mut action| {
                action.line = Some(line_number as u32);
                action
            }));
            if actions.len() > MAX_EDIT_ACTIONS {
                return Err(error(
                    "SCRIPT_LIMIT",
                    format!("A command file may contain at most {MAX_EDIT_ACTIONS} edit actions."),
                ));
            }
        }
        client.edit(actions)
    }

    fn render(value: &Value) -> String {
        if let Some(script) = value["result"]["script"].as_str() {
            return script.to_owned();
        }
        if let Some(e) = value.get("error") {
            format!(
                "{}: {}",
                e["code"].as_str().unwrap_or("ERROR"),
                e["message"].as_str().unwrap_or("request failed")
            )
        } else if let Some(result) = value.get("result") {
            let mut lines = Vec::new();
            if let Some(fields) = result.as_object() {
                for (key, value) in fields {
                    if let Some(rows) = value
                        .as_array()
                        .filter(|rows| !rows.is_empty() && rows[0].is_object())
                    {
                        lines.push(format!("{key}:"));
                        let columns: Vec<_> = rows
                            .iter()
                            .filter_map(Value::as_object)
                            .flat_map(|row| row.keys())
                            .collect::<std::collections::BTreeSet<_>>()
                            .into_iter()
                            .collect();
                        lines.push(format!(
                            "  {}",
                            columns
                                .iter()
                                .map(|c| c.as_str())
                                .collect::<Vec<_>>()
                                .join("  ")
                        ));
                        for row in rows {
                            lines.push(format!(
                                "  {}",
                                columns
                                    .iter()
                                    .map(|c| display_cell(&row[*c]))
                                    .collect::<Vec<_>>()
                                    .join("  ")
                            ));
                        }
                    } else if value.is_object() {
                        lines.push(format!("{key}:"));
                        lines.extend(
                            render(&json!({"result":value}))
                                .lines()
                                .map(|line| format!("  {line}")),
                        );
                    } else {
                        lines.push(format!("{key}: {}", display_cell(value)));
                    }
                }
                lines.join("\n")
            } else {
                display_cell(result)
            }
        } else {
            display_cell(value)
        }
    }
    fn display_cell(value: &Value) -> String {
        match value {
            Value::String(s) => s.clone(),
            Value::Null => "-".into(),
            Value::Array(items) => items
                .iter()
                .map(display_cell)
                .collect::<Vec<_>>()
                .join(", "),
            Value::Object(fields) => fields
                .iter()
                .map(|(key, value)| format!("{key}={}", display_cell(value)))
                .collect::<Vec<_>>()
                .join(", "),
            _ => value.to_string(),
        }
    }
    fn bash_completions() -> String {
        fn commands(command: &clap::Command, path: String, items: &mut Vec<(String, String)>) {
            let choices = command
                .get_subcommands()
                .map(|c| c.get_name().to_owned())
                .chain(
                    command
                        .get_arguments()
                        .filter_map(|arg| arg.get_long().map(|long| format!("--{long}"))),
                )
                .collect::<Vec<_>>()
                .join(" ");
            items.push((path.clone(), choices));
            for child in command.get_subcommands() {
                commands(child, format!("{path} {}", child.get_name()), items);
            }
        }
        let mut items = Vec::new();
        commands(&Cli::command(), "aitd".into(), &mut items);
        let paths = items
            .iter()
            .map(|(path, _)| format!("'{path}'"))
            .collect::<Vec<_>>()
            .join("|");
        let mut script = format!(
            "# Generated by aitd completions bash\n_aitd_complete() {{\n  local path=aitd word words cur=${{COMP_WORDS[COMP_CWORD]}}\n  for word in \"${{COMP_WORDS[@]:1:COMP_CWORD-1}}\"; do\n    case \"$path $word\" in {paths}) path=\"$path $word\" ;; esac\n  done\n  case \"$path\" in\n"
        );
        for (path, choices) in items {
            script.push_str(&format!("    '{path}') words='{choices}' ;;\n"));
        }
        script.push_str("  esac\n  COMPREPLY=( $(compgen -W \"$words\" -- \"$cur\") )\n}\ncomplete -F _aitd_complete aitd\n");
        script
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn command_files_preserve_shell_quoted_text() {
            assert_eq!(
                split_line(r#"object set Text 'value=123' 'path=C:\work' value=\"quoted\" ''"#)
                    .unwrap(),
                [
                    "object",
                    "set",
                    "Text",
                    "value=123",
                    r"path=C:\work",
                    "value=\"quoted\"",
                    ""
                ]
            );
            assert_eq!(
                split_line(r#"object set Text "value=hello \n world""#).unwrap()[3],
                r"value=hello \n world"
            );
            assert!(split_line("'unterminated").is_err());
            assert!(continues("object set Text value=hello\\"));
            assert!(!continues("object set Text 'value=hello\\'"));
        }
    }

    pub fn run() -> ExitCode {
        let args: Vec<_> = std::env::args_os().collect();
        let machine = args.iter().any(|arg| arg == "--json");
        let cli = match Cli::try_parse_from(&args) {
            Ok(cli) => cli,
            Err(err)
                if matches!(
                    err.kind(),
                    clap::error::ErrorKind::DisplayHelp | clap::error::ErrorKind::DisplayVersion
                ) =>
            {
                let _ = err.print();
                return ExitCode::SUCCESS;
            }
            Err(err) => {
                if machine {
                    eprintln!("{}", error("INVALID_USAGE", err.to_string()));
                } else {
                    let _ = err.print();
                }
                return ExitCode::from(2);
            }
        };
        let machine = cli.json;
        match execute(cli) {
            Ok(value) => {
                if machine {
                    println!("{}", value)
                } else {
                    println!("{}", render(&value))
                }
                ExitCode::SUCCESS
            }
            Err(value) => {
                if machine {
                    eprintln!("{}", value)
                } else {
                    eprintln!("{}", render(&value))
                }
                let code = value["error"]["code"].as_str().unwrap_or("");
                match code {
                    "INVALID_USAGE" | "INVALID_ARGUMENTS" => ExitCode::from(2),
                    "PROJECT_NOT_OPEN" => ExitCode::from(3),
                    "SOURCE_CHANGED" => ExitCode::from(4),
                    "VALIDATION_FAILED" => ExitCode::from(5),
                    _ => ExitCode::FAILURE,
                }
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() -> std::process::ExitCode {
    native::run()
}
#[cfg(target_arch = "wasm32")]
fn main() {}
