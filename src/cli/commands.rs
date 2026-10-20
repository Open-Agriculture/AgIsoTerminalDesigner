//! Shared readable CLI command definitions, parsing, and edit translation.
use crate::{
    cli::editing::{
        EditAction, EditActionKind, ParameterInput, RelationshipEdit, MAX_EDIT_ACTIONS,
    },
    components::{ComponentParameterKind, ComponentParameterTarget},
    object_properties::PropertyKey,
    operations::{ObjectTarget, Operation},
};
use ag_iso_stack::object_pool::{
    object_attributes::{DataCodeType, PictureGraphicFormat},
    ObjectType,
};
use clap::{Parser, Subcommand};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
};

#[derive(Debug, Parser)]
#[command(
    name = "aitd",
    about = "Inspect and edit a standalone working project in this shell"
)]
pub struct Cli {
    /// Validate an edit without changing the project.
    #[arg(long, global = true)]
    pub check: bool,
    /// Print structured JSON instead of readable text.
    #[arg(long, global = true)]
    pub json: bool,
    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand, Clone, serde::Serialize, serde::Deserialize)]
pub enum Command {
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
        #[arg(allow_hyphen_values = true, value_parser = ["bash"])]
        shell: String,
    },
    /// Inspect a mask or container hierarchy.
    Tree {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        root: String,
        #[arg(long, default_value_t = 6)]
        depth: usize,
    },
    /// Read a design context.
    Context {
        #[arg(allow_hyphen_values = true, long, default_value = "active_mask")]
        scope: String,
        #[arg(value_name = "NAME", long, allow_hyphen_values = true)]
        root: Option<String>,
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
    /// Reassign automatic protocol object IDs.
    Id {
        #[command(subcommand)]
        command: IdCommand,
    },
    /// Read writable properties and relationship rules.
    Schema {
        object_types: Vec<String>,
        /// Return only the requested properties.
        #[arg(allow_hyphen_values = true, long, value_delimiter = ',')]
        properties: Vec<String>,
    },
    /// Validate the current project.
    Validate,
    /// Render a preview to a PNG file.
    Preview {
        #[arg(long, allow_hyphen_values = true)]
        mask: Option<String>,
        #[arg(allow_hyphen_values = true, long)]
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
        #[arg(allow_hyphen_values = true, long)]
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
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        root: String,
        #[arg(allow_hyphen_values = true, long,default_value="all",value_parser=["top","bottom","left","right","center","top_left","top_right","bottom_left","bottom_right","all"])]
        region: String,
        #[arg(allow_hyphen_values = true, long)]
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
    /// Edit readable instructions in a Macro object.
    Macro {
        #[command(subcommand)]
        command: MacroCommand,
    },
    /// Add or edit object labels.
    Label {
        #[command(subcommand)]
        command: LabelCommand,
    },
}
#[derive(Debug, Subcommand, Clone, serde::Serialize, serde::Deserialize)]
pub enum BackupCommand {
    List {
        #[arg(allow_hyphen_values = true, long)]
        project: Option<PathBuf>,
    },
}
#[derive(Debug, Subcommand, Clone, serde::Serialize, serde::Deserialize)]
pub enum ObjectCommand {
    /// Set readable text with an explicit VT character encoding.
    TextSet {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        id: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
        #[arg(allow_hyphen_values = true)]
        text: String,
        #[arg(long, default_value = "utf16le", allow_hyphen_values = true)]
        encoding: String,
    },
    List {
        #[arg(allow_hyphen_values = true, long)]
        query: Option<String>,
        #[arg(allow_hyphen_values = true, long = "type")]
        object_type: Option<String>,
        #[arg(value_name = "NAME", allow_hyphen_values = true, long)]
        parent: Option<String>,
        #[arg(long, default_value_t = 0)]
        offset: usize,
        #[arg(long, default_value_t = 100)]
        limit: usize,
        /// Read all matching pages.
        #[arg(long)]
        all: bool,
    },
    Get {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
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
        #[arg(allow_hyphen_values = true)]
        object_type: String,
        #[arg(allow_hyphen_values = true)]
        name: String,
        /// Keep this protocol object ID fixed instead of allocating it.
        #[arg(value_name = "ID", long)]
        override_id: Option<String>,
    },
    Set {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        id: String,
        #[arg(required = true)]
        assignments: Vec<String>,
    },
    Rename {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        id: String,
        #[arg(allow_hyphen_values = true)]
        name: String,
    },
    /// Set editor metadata without changing pool properties.
    Metadata {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        id: String,
        #[arg(allow_hyphen_values = true, long, conflicts_with = "clear_name")]
        name: Option<String>,
        #[arg(long)]
        clear_name: bool,
        #[arg(allow_hyphen_values = true, long, conflicts_with = "clear_description")]
        description: Option<String>,
        #[arg(long)]
        clear_description: bool,
        #[arg(allow_hyphen_values = true, long, conflicts_with = "clear_notes")]
        notes: Option<String>,
        #[arg(long)]
        clear_notes: bool,
        #[arg(allow_hyphen_values = true, long = "tag", action = clap::ArgAction::Append, conflicts_with = "clear_tags")]
        tags: Vec<String>,
        #[arg(long)]
        clear_tags: bool,
    },
    OverrideId {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        object: String,
        #[arg(value_name = "ID|automatic", allow_hyphen_values = true)]
        assignment: String,
    },
    Delete {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        id: String,
    },
    Order {
        #[arg(value_name = "NAMES", allow_hyphen_values = true)]
        ids: Vec<String>,
    },
    Duplicate {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        id: String,
        #[arg(allow_hyphen_values = true, long)]
        name: Option<String>,
    },
    Field {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        id: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
        #[arg(required = true)]
        assignments: Vec<String>,
    },
    Array {
        #[command(subcommand)]
        command: ObjectArrayCommand,
    },
    BytesImport {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        id: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
        #[arg(allow_hyphen_values = true)]
        file: PathBuf,
    },
    /// Save a byte-valued property without JSON encoding.
    BytesExport {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        id: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
        #[arg(allow_hyphen_values = true, long)]
        output: PathBuf,
        #[arg(long)]
        force: bool,
    },
}
#[derive(Debug, Subcommand, Clone, serde::Serialize, serde::Deserialize)]
pub enum ObjectArrayCommand {
    Set {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        object: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
        index: u16,
        #[arg(allow_hyphen_values = true, required = true)]
        values: Vec<String>,
    },
    Append {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        object: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
        #[arg(allow_hyphen_values = true, required = true)]
        values: Vec<String>,
    },
    Remove {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        object: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
        index: u16,
    },
    Clear {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        object: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
    },
}
#[derive(Debug, Subcommand, Clone, serde::Serialize, serde::Deserialize)]
pub enum ComponentCommand {
    /// Set readable node text with an explicit VT character encoding.
    NodeTextSet {
        #[arg(value_name = "COMPONENT_NAME", allow_hyphen_values = true)]
        component_id: String,
        #[arg(value_name = "NODE_NAME", allow_hyphen_values = true)]
        node_id: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
        #[arg(allow_hyphen_values = true)]
        text: String,
        #[arg(long, default_value = "utf16le", allow_hyphen_values = true)]
        encoding: String,
    },
    List {
        #[arg(allow_hyphen_values = true, long)]
        query: Option<String>,
        #[arg(allow_hyphen_values = true, long)]
        category: Option<String>,
        #[arg(long, default_value_t = 50)]
        limit: u8,
    },
    Get {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        id: String,
        #[arg(long)]
        nodes: bool,
    },
    Instantiate {
        #[arg(value_name = "COMPONENT_NAME", allow_hyphen_values = true)]
        component_id: String,
        #[arg(value_name = "OBJECT_NAME", allow_hyphen_values = true)]
        parent: String,
        #[arg(allow_negative_numbers = true)]
        x: i32,
        #[arg(allow_negative_numbers = true)]
        y: i32,
    },
    Move {
        #[arg(value_name = "INSTANCE_NAME", allow_hyphen_values = true)]
        instance_id: String,
        #[arg(allow_negative_numbers = true)]
        x: i32,
        #[arg(allow_negative_numbers = true)]
        y: i32,
    },
    Duplicate {
        #[arg(value_name = "INSTANCE_NAME", allow_hyphen_values = true)]
        instance_id: String,
        #[arg(allow_negative_numbers = true)]
        x: i32,
        #[arg(allow_negative_numbers = true)]
        y: i32,
    },
    Delete {
        #[arg(value_name = "INSTANCE_NAME", allow_hyphen_values = true)]
        instance_id: String,
    },
    Detach {
        #[arg(value_name = "INSTANCE_NAME", allow_hyphen_values = true)]
        instance_id: String,
    },
    Override {
        #[arg(value_name = "INSTANCE_NAME", allow_hyphen_values = true)]
        instance_id: String,
        #[arg(value_name = "PARAMETER_NAME", allow_hyphen_values = true)]
        parameter: String,
        #[arg(allow_hyphen_values = true)]
        value: String,
    },
    Reset {
        #[arg(value_name = "INSTANCE_NAME", allow_hyphen_values = true)]
        instance_id: String,
        #[arg(value_name = "PARAMETER_NAME", allow_hyphen_values = true)]
        parameter: String,
    },
    DefinitionRename {
        #[arg(value_name = "COMPONENT_NAME", allow_hyphen_values = true)]
        component_id: String,
        #[arg(allow_hyphen_values = true)]
        name: String,
    },
    DefinitionDuplicate {
        #[arg(value_name = "COMPONENT_NAME", allow_hyphen_values = true)]
        component_id: String,
    },
    DefinitionDelete {
        #[arg(value_name = "COMPONENT_NAME", allow_hyphen_values = true)]
        component_id: String,
    },
    NodeSet {
        #[arg(value_name = "COMPONENT_NAME", allow_hyphen_values = true)]
        component_id: String,
        #[arg(value_name = "NODE_NAME", allow_hyphen_values = true)]
        node_id: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
        #[arg(allow_hyphen_values = true)]
        value: String,
    },
    /// Assign a node reference using `node:NAME`, `object:NAME`, or `null`.
    NodeReference {
        #[arg(value_name = "COMPONENT_NAME", allow_hyphen_values = true)]
        component_id: String,
        #[arg(value_name = "NODE_NAME", allow_hyphen_values = true)]
        node_id: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
        #[arg(allow_hyphen_values = true)]
        target: String,
    },
    ParameterRemove {
        #[arg(value_name = "COMPONENT_NAME", allow_hyphen_values = true)]
        component_id: String,
        #[arg(value_name = "PARAMETER_NAME", allow_hyphen_values = true)]
        parameter_id: String,
    },
    Capture {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        root: String,
        #[arg(allow_hyphen_values = true)]
        name: String,
    },
    ParameterAdd {
        #[arg(value_name = "COMPONENT_NAME", allow_hyphen_values = true)]
        component_id: String,
        #[arg(allow_hyphen_values = true)]
        name: String,
        #[arg(allow_hyphen_values = true)]
        target_node: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
        #[arg(allow_hyphen_values = true)]
        kind: String,
        #[arg(allow_hyphen_values = true)]
        default_value: String,
    },
    ParameterSet {
        #[arg(value_name = "COMPONENT_NAME", allow_hyphen_values = true)]
        component_id: String,
        #[arg(value_name = "PARAMETER_NAME", allow_hyphen_values = true)]
        parameter_id: String,
        #[arg(allow_hyphen_values = true)]
        name: String,
        #[arg(allow_hyphen_values = true)]
        target_node: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
        #[arg(allow_hyphen_values = true)]
        kind: String,
        #[arg(allow_hyphen_values = true)]
        default_value: String,
    },
    /// Create a component definition with its root node.
    DefinitionCreate {
        #[arg(allow_hyphen_values = true)]
        name: String,
        #[arg(allow_hyphen_values = true)]
        root_name: String,
        #[arg(allow_hyphen_values = true)]
        root_type: String,
        #[arg(allow_hyphen_values = true, long, default_value = "")]
        description: String,
        #[arg(allow_hyphen_values = true, long = "category")]
        category_path: Vec<String>,
        #[arg(allow_hyphen_values = true, long = "tag")]
        tags: Vec<String>,
    },
    DefinitionMetadata {
        #[arg(value_name = "COMPONENT_NAME", allow_hyphen_values = true)]
        component_id: String,
        #[arg(allow_hyphen_values = true, long)]
        name: Option<String>,
        #[arg(allow_hyphen_values = true, long)]
        description: Option<String>,
        #[arg(allow_hyphen_values = true, long = "category")]
        category_path: Option<Vec<String>>,
        #[arg(allow_hyphen_values = true, long = "tag")]
        tags: Option<Vec<String>>,
        #[arg(long)]
        revision: Option<u64>,
    },
    NodeCreate {
        #[arg(value_name = "COMPONENT_NAME", allow_hyphen_values = true)]
        component_id: String,
        #[arg(allow_hyphen_values = true)]
        name: String,
        #[arg(allow_hyphen_values = true)]
        object_type: String,
    },
    NodeOrderSet {
        #[arg(value_name = "COMPONENT_NAME", allow_hyphen_values = true)]
        component_id: String,
        #[arg(allow_hyphen_values = true, required = true)]
        nodes: Vec<String>,
    },
    NodeRename {
        #[arg(value_name = "COMPONENT_NAME", allow_hyphen_values = true)]
        component_id: String,
        #[arg(value_name = "NODE_NAME", allow_hyphen_values = true)]
        node_id: String,
        #[arg(allow_hyphen_values = true)]
        name: String,
    },
    NodePropertySet {
        #[arg(value_name = "COMPONENT_NAME", allow_hyphen_values = true)]
        component_id: String,
        #[arg(value_name = "NODE_NAME", allow_hyphen_values = true)]
        node_id: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
        /// A typed JSON value such as `"Hello"`, `true`, or `17`.
        #[arg(allow_hyphen_values = true)]
        value: String,
    },
    /// Assign indexed PNG image data to a PictureGraphic component node.
    NodeAssignFile {
        #[arg(value_name = "COMPONENT_NAME", allow_hyphen_values = true)]
        component_id: String,
        #[arg(value_name = "NODE_NAME", allow_hyphen_values = true)]
        node_id: String,
        file: PathBuf,
        #[arg(long, value_parser = ["monochrome", "four_bit", "eight_bit"])]
        format: String,
        #[arg(long, value_parser = ["raw", "run_length"])]
        encoding: String,
    },
    /// Append one readable instruction to a component Macro node.
    NodeMacroAppend {
        #[arg(value_name = "COMPONENT_NAME", allow_hyphen_values = true)]
        component_id: String,
        #[arg(value_name = "NODE_NAME", allow_hyphen_values = true)]
        node_id: String,
        #[arg(allow_hyphen_values = true)]
        instruction: String,
        #[arg(allow_hyphen_values = true, required = true)]
        arguments: Vec<String>,
    },
    /// Set a component Macro node's Commands property to an empty stream.
    NodeMacroClear {
        #[arg(value_name = "COMPONENT_NAME", allow_hyphen_values = true)]
        component_id: String,
        #[arg(value_name = "NODE_NAME", allow_hyphen_values = true)]
        node_id: String,
    },
    NodeReferenceSet {
        #[arg(value_name = "COMPONENT_NAME", allow_hyphen_values = true)]
        component_id: String,
        #[arg(value_name = "NODE_NAME", allow_hyphen_values = true)]
        node_id: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
        /// A typed ComponentReference JSON value.
        #[arg(allow_hyphen_values = true)]
        target: String,
    },
    NodeChildrenSet {
        #[arg(value_name = "COMPONENT_NAME", allow_hyphen_values = true)]
        component_id: String,
        #[arg(value_name = "NODE_NAME", allow_hyphen_values = true)]
        node_id: String,
        /// Ordered child specifications `NODE_NAME:X:Y`.
        #[arg(allow_hyphen_values = true, required = true)]
        children: Vec<String>,
    },
    NodeMacroReferencesSet {
        #[arg(value_name = "COMPONENT_NAME", allow_hyphen_values = true)]
        component_id: String,
        #[arg(value_name = "NODE_NAME", allow_hyphen_values = true)]
        node_id: String,
        /// Typed JSON array of `{ "event": ..., "target": ... }` records.
        #[arg(allow_hyphen_values = true)]
        references: String,
    },
    NodeObjectListSet {
        #[arg(value_name = "COMPONENT_NAME", allow_hyphen_values = true)]
        component_id: String,
        #[arg(value_name = "NODE_NAME", allow_hyphen_values = true)]
        node_id: String,
        /// Typed ComponentObjectList JSON, or `null` to clear.
        #[arg(allow_hyphen_values = true)]
        object_list: String,
    },
    ParameterAddTyped {
        #[arg(value_name = "COMPONENT_NAME", allow_hyphen_values = true)]
        component_id: String,
        #[arg(allow_hyphen_values = true)]
        name: String,
        #[arg(allow_hyphen_values = true)]
        target_node: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
        #[arg(allow_hyphen_values = true)]
        kind: String,
        #[arg(allow_hyphen_values = true)]
        default_value: String,
        #[arg(allow_hyphen_values = true, long)]
        description: Option<String>,
    },
    ParameterSetTyped {
        #[arg(value_name = "COMPONENT_NAME", allow_hyphen_values = true)]
        component_id: String,
        #[arg(value_name = "PARAMETER_NAME", allow_hyphen_values = true)]
        parameter: String,
        #[arg(allow_hyphen_values = true)]
        name: String,
        #[arg(allow_hyphen_values = true)]
        target_node: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
        #[arg(allow_hyphen_values = true)]
        kind: String,
        #[arg(allow_hyphen_values = true)]
        default_value: String,
        #[arg(allow_hyphen_values = true, long)]
        description: Option<String>,
    },
    InstanceCreate {
        #[arg(allow_hyphen_values = true)]
        name: String,
        #[arg(value_name = "COMPONENT_NAME", allow_hyphen_values = true)]
        component_id: String,
        #[arg(value_name = "OBJECT_NAME", allow_hyphen_values = true)]
        parent: String,
        #[arg(allow_negative_numbers = true)]
        x: i32,
        #[arg(allow_negative_numbers = true)]
        y: i32,
        #[arg(long)]
        child_index: Option<usize>,
        #[arg(allow_hyphen_values = true, long, default_value = "{}")]
        overrides: String,
        #[arg(
            value_name = "NODE_NAME=ID",
            allow_hyphen_values = true,
            long = "compiled-override"
        )]
        compiled_ids: Vec<String>,
    },
    InstanceMetadata {
        #[arg(value_name = "INSTANCE_NAME", allow_hyphen_values = true)]
        instance_id: String,
        #[arg(long)]
        child_index: Option<usize>,
        #[arg(
            value_name = "NODE_NAME=ID",
            allow_hyphen_values = true,
            long = "compiled-override"
        )]
        compiled_ids: Vec<String>,
    },
    OverrideSet {
        #[arg(value_name = "INSTANCE_NAME", allow_hyphen_values = true)]
        instance_id: String,
        #[arg(value_name = "PARAMETER_NAME", allow_hyphen_values = true)]
        parameter_id: String,
        #[arg(allow_hyphen_values = true)]
        value: String,
    },
}
#[derive(Debug, Subcommand, Clone, serde::Serialize, serde::Deserialize)]
pub enum ProjectCommand {
    /// Load a project into this shell session.
    Load {
        #[arg(allow_hyphen_values = true)]
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
        #[arg(allow_hyphen_values = true)]
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
        #[arg(allow_hyphen_values = true)]
        object_type: String,
        start: u16,
        end: u16,
    },
    IdRangeReset,
}
#[derive(Debug, Subcommand, Clone, serde::Serialize, serde::Deserialize)]
pub enum ProjectSettingsCommand {
    Show,
    /// Change settings with typed key=value assignments.
    Set {
        #[arg(required = true)]
        assignments: Vec<String>,
    },
}
#[derive(Debug, Subcommand, Clone, serde::Serialize, serde::Deserialize)]
pub enum InstanceCommand {
    List,
    Get {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        id: String,
    },
}
#[derive(Debug, Subcommand, Clone, serde::Serialize, serde::Deserialize)]
pub enum PlacementCommand {
    Add {
        #[arg(value_name = "OBJECT_NAME", allow_hyphen_values = true)]
        parent: String,
        #[arg(allow_hyphen_values = true)]
        child: String,
        #[arg(allow_negative_numbers = true)]
        x: i16,
        #[arg(allow_negative_numbers = true)]
        y: i16,
        #[arg(long)]
        occurrence: Option<usize>,
    },
    Move {
        #[arg(value_name = "OBJECT_NAME", allow_hyphen_values = true)]
        parent: String,
        #[arg(allow_hyphen_values = true)]
        child: String,
        #[arg(allow_negative_numbers = true)]
        x: i16,
        #[arg(allow_negative_numbers = true)]
        y: i16,
        #[arg(long)]
        occurrence: Option<usize>,
    },
    Remove {
        #[arg(value_name = "OBJECT_NAME", allow_hyphen_values = true)]
        parent: String,
        #[arg(allow_hyphen_values = true)]
        child: String,
        #[arg(long)]
        occurrence: Option<usize>,
    },
    Order {
        #[arg(value_name = "OBJECT_NAME", allow_hyphen_values = true)]
        parent: String,
        #[arg(allow_hyphen_values = true)]
        children: Vec<String>,
    },
}
#[derive(Debug, Subcommand, Clone, serde::Serialize, serde::Deserialize)]
pub enum GraphicsCommand {
    Get {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        id: String,
    },
    /// Save an available asset or PictureGraphic as a PNG.
    Export {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        id: String,
        #[arg(allow_hyphen_values = true, long)]
        output: PathBuf,
        #[arg(long)]
        force: bool,
    },
    List {
        #[arg(allow_hyphen_values = true, long)]
        query: Option<String>,
        #[arg(allow_hyphen_values = true, long, default_value = "all")]
        status: String,
        #[arg(long, default_value_t = 50)]
        limit: u8,
    },
    Import {
        #[arg(allow_hyphen_values = true)]
        file: PathBuf,
        #[arg(allow_hyphen_values = true, long)]
        name: Option<String>,
        #[arg(allow_hyphen_values = true, long)]
        description: Option<String>,
        #[arg(allow_hyphen_values = true, long = "tag")]
        tags: Vec<String>,
        /// Restore the original download filename after importing a converted PNG.
        #[arg(allow_hyphen_values = true, long)]
        original_name: Option<String>,
        /// Clear the original download filename metadata.
        #[arg(long)]
        clear_original_name: bool,
    },
    Assign {
        #[arg(value_name = "GRAPHIC_NAME", allow_hyphen_values = true)]
        asset_id: String,
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        object: String,
    },
    /// Assign an indexed PNG directly, retaining palette indices and requested encoding.
    AssignFile {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        object: String,
        #[arg(allow_hyphen_values = true)]
        file: PathBuf,
        #[arg(allow_hyphen_values = true, long, value_parser = ["monochrome", "four_bit", "eight_bit"])]
        format: String,
        #[arg(allow_hyphen_values = true, long, value_parser = ["raw", "run_length"])]
        encoding: String,
    },
    CreatePicture {
        #[arg(value_name = "GRAPHIC_NAME", allow_hyphen_values = true)]
        asset_id: String,
    },
    Rename {
        #[arg(value_name = "GRAPHIC_NAME", allow_hyphen_values = true)]
        asset_id: String,
        #[arg(allow_hyphen_values = true)]
        name: String,
    },
    Delete {
        #[arg(value_name = "GRAPHIC_NAME", allow_hyphen_values = true)]
        asset_id: String,
    },
}
#[derive(Debug, Subcommand, Clone, serde::Serialize, serde::Deserialize)]
pub enum ExportCommand {
    /// Validate and publish a new directory containing project.aitp and project.iop.
    Project {
        #[arg(allow_hyphen_values = true, long)]
        output_dir: PathBuf,
    },
    Iop {
        #[arg(allow_hyphen_values = true, long)]
        output: PathBuf,
        #[arg(long)]
        force: bool,
    },
    Header {
        #[arg(allow_hyphen_values = true, long)]
        output: PathBuf,
        #[arg(long)]
        force: bool,
    },
    Bundle {
        #[arg(allow_hyphen_values = true, long)]
        iop: PathBuf,
        #[arg(allow_hyphen_values = true, long)]
        header: PathBuf,
        #[arg(long)]
        force: bool,
    },
}

#[derive(Debug, Subcommand, Clone, serde::Serialize, serde::Deserialize)]
pub enum IdCommand {
    Reassign {
        #[arg(long)]
        include_overrides: bool,
    },
}
#[derive(Debug, Subcommand, Clone, serde::Serialize, serde::Deserialize)]
pub enum ReferenceCommand {
    Set {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        object: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
        #[arg(allow_hyphen_values = true)]
        target: String,
    },
    Clear {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        object: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
    },
}
#[derive(Debug, Subcommand, Clone, serde::Serialize, serde::Deserialize)]
pub enum PointerCommand {
    Set {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        object: String,
        #[arg(allow_hyphen_values = true)]
        targets: Vec<String>,
    },
    Clear {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        object: String,
    },
}
#[derive(Debug, Subcommand, Clone, serde::Serialize, serde::Deserialize)]
pub enum ListCommand {
    Set {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        object: String,
        #[arg(allow_hyphen_values = true)]
        targets: Vec<String>,
    },
    Insert {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        object: String,
        index: usize,
        #[arg(allow_hyphen_values = true)]
        target: String,
    },
    Remove {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        object: String,
        index: usize,
    },
    Clear {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        object: String,
    },
}
#[derive(Debug, Subcommand, Clone, serde::Serialize, serde::Deserialize)]
pub enum EventCommand {
    Bind {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        object: String,
        #[arg(required = true)]
        bindings: Vec<String>,
    },
    Set {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        object: String,
        #[arg(required = true)]
        bindings: Vec<String>,
    },
    Unbind {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        object: String,
        #[arg(allow_hyphen_values = true)]
        event: String,
    },
    Order {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        object: String,
        #[arg(required = true)]
        events: Vec<String>,
    },
    Clear {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        object: String,
    },
}

#[derive(Debug, Subcommand, Clone, serde::Serialize, serde::Deserialize)]
pub enum MacroCommand {
    /// Append one named instruction to a Macro object.
    Append {
        #[arg(allow_hyphen_values = true)]
        target: String,
        #[arg(allow_hyphen_values = true)]
        instruction: String,
        #[arg(allow_hyphen_values = true)]
        arguments: Vec<String>,
    },
}
#[derive(Debug, Subcommand, Clone, serde::Serialize, serde::Deserialize)]
pub enum LabelCommand {
    Add {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        object: String,
        #[arg(required = true)]
        fields: Vec<String>,
    },
    Set {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        object: String,
        #[arg(required = true)]
        fields: Vec<String>,
    },
    Remove {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        object: String,
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        id: String,
    },
    Clear {
        #[arg(value_name = "NAME", allow_hyphen_values = true)]
        object: String,
    },
}

pub fn cli_id(value: &str) -> Result<u16, Value> {
    let (digits, radix) = value
        .strip_prefix("0x")
        .map(|v| (v, 16))
        .unwrap_or((value, 10));
    u16::from_str_radix(digits, radix)
        .map_err(|_| error("INVALID_USAGE", format!("'{value}' must be an object ID")))
}

/// Build a CLI command that reproduces metadata for the object with this name.
pub fn object_metadata_tokens(name: &str, info: &crate::ObjectInfo) -> Vec<String> {
    let mut tokens = vec!["object".into(), "metadata".into(), name.to_owned()];
    append_optional_metadata(&mut tokens, "name", info.name.as_deref());
    append_optional_metadata(&mut tokens, "description", info.description.as_deref());
    append_optional_metadata(&mut tokens, "notes", info.notes.as_deref());
    if info.tags.is_empty() {
        tokens.push("--clear-tags".into());
    } else {
        for tag in &info.tags {
            tokens.extend(["--tag".into(), tag.clone()]);
        }
    }
    tokens
}

fn append_optional_metadata(tokens: &mut Vec<String>, field: &str, value: Option<&str>) {
    if let Some(value) = value {
        tokens.extend([format!("--{field}"), value.to_owned()]);
    } else {
        tokens.push(format!("--clear-{field}"));
    }
}

/// Build a CLI command for one decoded Macro instruction targeting this name.
pub fn macro_instruction_tokens(
    target: &str,
    instruction: &crate::macro_authoring::MacroInstruction,
) -> Vec<String> {
    let mut tokens = vec![
        "macro".into(),
        "append".into(),
        target.to_owned(),
        instruction.name.clone(),
    ];
    tokens.extend(instruction.arguments.iter().cloned());
    tokens
}

/// Convert a Clap-validated command into the shared edit action.
/// File inputs resolve relative to `base`, which is cwd for shell commands
/// and the command file's parent for `run --file`.
fn component_target(
    project: Option<&crate::EditorProject>,
    name: &str,
) -> Result<crate::components::ComponentId, Value> {
    project
        .map(|project| crate::cli::names::component_id(project, name))
        .unwrap_or_else(|| {
            Err(format!(
                "Cannot resolve component name '{name}' without a project"
            ))
        })
        .map_err(|message| error("INVALID_USAGE", message))
}

fn node_target(
    project: Option<&crate::EditorProject>,
    component: &str,
    name: &str,
) -> Result<crate::components::ComponentNodeId, Value> {
    project
        .map(|project| crate::cli::names::node_id(project, component, name))
        .unwrap_or_else(|| {
            Err(format!(
                "Cannot resolve component node '{name}' without a project"
            ))
        })
        .map_err(|message| error("INVALID_USAGE", message))
}

fn instance_target(
    project: Option<&crate::EditorProject>,
    name: &str,
) -> Result<crate::components::ComponentInstanceId, Value> {
    project
        .map(|project| crate::cli::names::instance_id(project, name))
        .unwrap_or_else(|| {
            Err(format!(
                "Cannot resolve component instance '{name}' without a project"
            ))
        })
        .map_err(|message| error("INVALID_USAGE", message))
}

fn parameter_target(
    project: Option<&crate::EditorProject>,
    component: &str,
    name: &str,
) -> Result<crate::components::ComponentParameterId, Value> {
    project
        .map(|project| crate::cli::names::parameter_id(project, component, name))
        .unwrap_or_else(|| {
            Err(format!(
                "Cannot resolve component parameter '{name}' without a project"
            ))
        })
        .map_err(|message| error("INVALID_USAGE", message))
}

fn graphic_target(
    project: Option<&crate::EditorProject>,
    name: &str,
) -> Result<crate::graphics::GraphicAssetId, Value> {
    project
        .map(|project| crate::cli::names::graphic_asset_id(project, name))
        .unwrap_or_else(|| {
            Err(format!(
                "Cannot resolve graphic name '{name}' without a project"
            ))
        })
        .map_err(|message| error("INVALID_USAGE", message))
}

fn typed_id<T: serde::de::DeserializeOwned>(value: &str, label: &str) -> Result<T, Value> {
    serde_json::from_value(json!(value))
        .map_err(|e| error("INVALID_USAGE", format!("invalid {label} '{value}': {e}")))
}
fn typed_object_type(value: &str) -> Result<ObjectType, Value> {
    typed_id(value, "object type")
}
fn typed_property(value: &str) -> Result<PropertyKey, Value> {
    value
        .parse()
        .map_err(|e: crate::object_properties::UnknownPropertyKey| {
            error("INVALID_USAGE", e.to_string())
        })
}
fn typed_property_path(path: &str) -> Result<(PropertyKey, String), Value> {
    let (property, field) = path.split_once('.').unwrap_or((path, ""));
    Ok((typed_property(property)?, field.to_owned()))
}
pub fn typed_target(value: &str) -> ObjectTarget {
    ObjectTarget::Name {
        name: value.strip_prefix("name:").unwrap_or(value).to_owned(),
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
fn author_component(
    action: crate::cli::component_authoring::ComponentAuthoringAction,
) -> Result<Option<Vec<EditAction>>, Value> {
    Ok(Some(vec![edit_action(EditActionKind::AuthorComponent {
        action,
    })]))
}
fn parse_typed_json<T: serde::de::DeserializeOwned>(value: &str, label: &str) -> Result<T, Value> {
    serde_json::from_str(value).map_err(|parse_error| {
        error(
            "INVALID_USAGE",
            format!("invalid {label} JSON: {parse_error}"),
        )
    })
}

fn parse_named_component_overrides(
    project: Option<&crate::EditorProject>,
    component: &str,
    raw: &str,
) -> Result<BTreeMap<crate::components::ComponentParameterId, Value>, Value> {
    let values: serde_json::Map<String, Value> = parse_typed_json(raw, "component overrides")?;
    values
        .into_iter()
        .map(|(name, value)| {
            let parameter_id = parameter_target(project, component, &name)?;
            let resolved = resolve_named_parameter_value(project, component, parameter_id, value)?;
            Ok((parameter_id, resolved))
        })
        .collect()
}

fn resolve_named_parameter_value(
    project: Option<&crate::EditorProject>,
    component: &str,
    parameter_id: crate::components::ComponentParameterId,
    value: Value,
) -> Result<Value, Value> {
    let project = project
        .ok_or_else(|| error("INVALID_USAGE", "Component values require a loaded project"))?;
    let component_id = component_target(Some(project), component)?;
    let parameter = project.component_source().definitions[&component_id]
        .parameters
        .iter()
        .find(|parameter| parameter.id == parameter_id)
        .ok_or_else(|| error("INVALID_USAGE", "Component parameter not found"))?;
    match parameter.kind {
        crate::components::ComponentParameterKind::Graphic => {
            let name = value.as_str().ok_or_else(|| {
                error(
                    "INVALID_USAGE",
                    "Graphic parameter values require a graphic name",
                )
            })?;
            let id = crate::cli::names::graphic_asset_id(project, name)
                .map_err(|m| error("INVALID_USAGE", m))?;
            serde_json::to_value(id).map_err(|e| error("INVALID_USAGE", e.to_string()))
        }
        crate::components::ComponentParameterKind::ObjectReference if !value.is_null() => {
            let target = if let Some(name) = value.as_str() {
                typed_target(name)
            } else {
                let target: ObjectTarget = serde_json::from_value(value).map_err(|e| {
                    error("INVALID_USAGE", format!("invalid object reference: {e}"))
                })?;
                if matches!(target, ObjectTarget::Id { .. }) {
                    return Err(error(
                        "INVALID_USAGE",
                        "Object reference values require an object name",
                    ));
                }
                target
            };
            serde_json::to_value(target).map_err(|e| error("INVALID_USAGE", e.to_string()))
        }
        _ => Ok(value),
    }
}

fn parse_parameter_default(
    project: Option<&crate::EditorProject>,
    kind: crate::components::ComponentParameterKind,
    raw: &str,
) -> Result<Value, Value> {
    use crate::components::ComponentParameterKind as K;
    match kind {
        K::Graphic => {
            let project = project.ok_or_else(|| {
                error(
                    "INVALID_USAGE",
                    "Graphic parameter requires a loaded project",
                )
            })?;
            let id = crate::cli::names::graphic_asset_id(project, raw)
                .map_err(|m| error("INVALID_USAGE", m))?;
            serde_json::to_value(id).map_err(|e| error("INVALID_USAGE", e.to_string()))
        }
        K::ObjectReference => {
            if raw == "null" {
                return Ok(Value::Null);
            }
            let target = if raw.trim_start().starts_with('{') {
                let target: ObjectTarget = parse_typed_json(raw, "object reference")?;
                if matches!(target, ObjectTarget::Id { .. }) {
                    return Err(error(
                        "INVALID_USAGE",
                        "Object reference values require an object name",
                    ));
                }
                target
            } else {
                typed_target(raw)
            };
            serde_json::to_value(target).map_err(|e| error("INVALID_USAGE", e.to_string()))
        }
        _ => parse_typed_json(raw, "parameter default"),
    }
}

fn validate_named_target(target: &ObjectTarget) -> Result<(), Value> {
    if matches!(target, ObjectTarget::Id { .. }) {
        Err(error("INVALID_USAGE", "CLI object targets must use names"))
    } else {
        Ok(())
    }
}

fn parse_named_macro_references(
    raw: &str,
) -> Result<Vec<crate::components::ComponentMacroReference>, Value> {
    let refs: Vec<crate::components::ComponentMacroReference> =
        parse_typed_json(raw, "component macro references")?;
    for reference in &refs {
        validate_named_target(&reference.target)?;
    }
    Ok(refs)
}

fn parse_named_object_list(
    raw: &str,
) -> Result<Option<crate::components::ComponentObjectList>, Value> {
    use crate::components::ComponentObjectList as L;
    let list: Option<L> = parse_typed_json(raw, "component object list")?;
    match &list {
        Some(L::Required(targets)) => {
            for target in targets {
                validate_named_target(target)?;
            }
        }
        Some(L::Nullable(targets)) => {
            for target in targets.iter().flatten() {
                validate_named_target(target)?;
            }
        }
        None => (),
    }
    Ok(list)
}

fn parse_component_reference(
    project: Option<&crate::EditorProject>,
    component_name: &str,
    raw: &str,
) -> Result<crate::components::ComponentReference, Value> {
    if raw == "null" {
        return Ok(crate::components::ComponentReference::Null);
    }
    if let Some(name) = raw.strip_prefix("node:") {
        return Ok(crate::components::ComponentReference::Internal(
            node_target(project, component_name, name)?,
        ));
    }
    if let Some(name) = raw.strip_prefix("parameter:") {
        return Ok(crate::components::ComponentReference::Parameter(
            parameter_target(project, component_name, name)?,
        ));
    }
    if let Some(name) = raw.strip_prefix("object:") {
        return Ok(crate::components::ComponentReference::ExternalObject(
            typed_target(name),
        ));
    }
    let mut value: Value = parse_typed_json(raw, "component reference")?;
    if let Some(map) = value.as_object_mut() {
        if let Some(Value::String(name)) = map.get("Internal").cloned() {
            map.insert(
                "Internal".into(),
                json!(node_target(project, component_name, &name)?),
            );
        } else if let Some(Value::String(name)) = map.get("Parameter").cloned() {
            map.insert(
                "Parameter".into(),
                json!(parameter_target(project, component_name, &name)?),
            );
        } else if let Some(target) = map.get("ExternalObject") {
            let is_numeric_id = target
                .as_object()
                .is_some_and(|target| target.get("id").is_some());
            if is_numeric_id {
                return Err(error(
                    "INVALID_USAGE",
                    "External component references require an object name",
                ));
            }
        }
    }
    serde_json::from_value(value)
        .map_err(|e| error("INVALID_USAGE", format!("invalid component reference: {e}")))
}
fn optional_target(value: &str) -> Option<ObjectTarget> {
    (value != "null").then(|| typed_target(value))
}
fn typed_event(value: &str) -> Result<ag_iso_stack::object_pool::object_attributes::Event, Value> {
    use ag_iso_stack::object_pool::object_attributes::Event;
    let event = typed_id(value, "event")?;
    if event == Event::Reserved {
        return Err(error("INVALID_USAGE", "Reserved is not an editable event"));
    }
    Ok(event)
}
pub fn command_edit_actions(
    command: &Command,
    base: &Path,
    read_asset: &dyn Fn(&Path) -> Result<Vec<u8>, String>,
) -> Result<Option<Vec<EditAction>>, Value> {
    if deferred_entity_command(command) {
        let mut assets = BTreeMap::new();
        if let Some(path) = command_asset_path(command) {
            let bytes =
                read_asset(&base.join(path)).map_err(|message| error("INPUT_FAILED", message))?;
            assets.insert(base.join(path).to_string_lossy().into_owned(), bytes);
        }
        return Ok(Some(vec![edit_action(EditActionKind::DeferredCommand {
            command: serde_json::to_value(command)
                .map_err(|e| error("INVALID_USAGE", e.to_string()))?,
            base: base.to_path_buf(),
            assets,
        })]));
    }
    command_edit_actions_now(command, base, read_asset, None)
}

fn deferred_entity_command(command: &Command) -> bool {
    match command {
        Command::Component { command } => !matches!(
            command,
            ComponentCommand::Get { .. } | ComponentCommand::List { .. }
        ),
        Command::Graphic { command } => !matches!(
            command,
            GraphicsCommand::Get { .. }
                | GraphicsCommand::List { .. }
                | GraphicsCommand::Export { .. }
        ),
        _ => false,
    }
}

fn command_asset_path(command: &Command) -> Option<&Path> {
    match command {
        Command::Component {
            command: ComponentCommand::NodeAssignFile { file, .. },
        } => Some(file),
        Command::Graphic {
            command: GraphicsCommand::Import { file, .. } | GraphicsCommand::AssignFile { file, .. },
        } => Some(file),
        _ => None,
    }
}

pub fn apply_deferred_command(
    project: &mut crate::EditorProject,
    command: serde_json::Value,
    base: PathBuf,
    assets: BTreeMap<String, Vec<u8>>,
) -> Result<Vec<EditAction>, Value> {
    let command: Command =
        serde_json::from_value(command).map_err(|e| error("INVALID_USAGE", e.to_string()))?;
    command_edit_actions_now(
        &command,
        &base,
        &|path| {
            assets
                .get(&path.to_string_lossy().into_owned())
                .cloned()
                .ok_or_else(|| format!("No preloaded CLI asset for {}", path.display()))
        },
        Some(project),
    )?
    .ok_or_else(|| error("INVALID_USAGE", "Deferred command is not an edit command"))
}

fn command_edit_actions_now(
    command: &Command,
    base: &Path,
    read_asset: &dyn Fn(&Path) -> Result<Vec<u8>, String>,
    project: Option<&crate::EditorProject>,
) -> Result<Option<Vec<EditAction>>, Value> {
    let one = |kind| Ok(Some(vec![edit_action(kind)]));
    match command {
        Command::Object { command } => match command {
            ObjectCommand::TextSet {
                id,
                property,
                text,
                encoding,
            } => one(EditActionKind::Operation {
                operation: Operation::SetProperty {
                    object: typed_target(id),
                    property: typed_property(property)?,
                    value: crate::cli::string_authoring::encode(text, encoding)
                        .map_err(|message| error("INVALID_USAGE", message))?,
                },
            }),
            ObjectCommand::Create {
                object_type,
                name,
                override_id,
            } => one(EditActionKind::ObjectCreate {
                object_type: typed_object_type(object_type)?,
                name: name.clone(),
                override_id: override_id.as_deref().map(cli_id).transpose()?,
            }),
            ObjectCommand::Set { id, assignments } => {
                let mut values = BTreeMap::new();
                for assignment in assignments {
                    let (property, value) = assignment.split_once('=').ok_or_else(|| {
                        error(
                            "INVALID_USAGE",
                            format!("expected property=value, got '{assignment}'"),
                        )
                    })?;
                    if values
                        .insert(typed_property(property)?, value.to_owned())
                        .is_some()
                    {
                        return Err(error(
                            "INVALID_USAGE",
                            format!("Repeated property '{property}'"),
                        ));
                    }
                }
                one(EditActionKind::SetProperties {
                    object: typed_target(id),
                    values,
                })
            }
            ObjectCommand::Rename { id, name } => one(EditActionKind::ObjectRename {
                object: typed_target(id),
                name: name.clone(),
            }),
            ObjectCommand::Metadata {
                id,
                name,
                clear_name,
                description,
                clear_description,
                notes,
                clear_notes,
                tags,
                clear_tags,
            } => one(EditActionKind::ObjectMetadataSet {
                object: typed_target(id),
                name: if *clear_name {
                    Some(None)
                } else {
                    name.clone().map(Some)
                },
                description: if *clear_description {
                    Some(None)
                } else {
                    description.clone().map(Some)
                },
                notes: if *clear_notes {
                    Some(None)
                } else {
                    notes.clone().map(Some)
                },
                tags: if *clear_tags {
                    Some(Vec::new())
                } else if tags.is_empty() {
                    None
                } else {
                    Some(tags.clone())
                },
                id_overridden: None,
            }),
            ObjectCommand::OverrideId { object, assignment } => {
                let (id_overridden, new_id) = if assignment == "automatic" {
                    (false, None)
                } else {
                    (true, Some(cli_id(assignment)?))
                };
                one(EditActionKind::ObjectIdOverride {
                    object: typed_target(object),
                    id_overridden,
                    new_id,
                })
            }
            ObjectCommand::Delete { id } => one(EditActionKind::Relationship {
                object: Some(typed_target(id)),
                operation: Operation::DeleteObject { object_id: 0 },
            }),
            ObjectCommand::Duplicate { id, name } => one(EditActionKind::ObjectDuplicate {
                object: typed_target(id),
                name: name.clone(),
            }),
            ObjectCommand::Order { ids } => one(EditActionKind::ObjectOrder {
                names: ids
                    .iter()
                    .map(|name| name.strip_prefix("name:").unwrap_or(name).to_owned())
                    .collect(),
            }),
            ObjectCommand::Field {
                id,
                property,
                assignments,
            } => {
                let (property, field) = property.split_once('.').unwrap_or((property, ""));
                one(EditActionKind::ObjectFieldSet {
                    object: typed_target(id),
                    property: typed_property(property)?,
                    field: field.to_owned(),
                    values: assignments.clone(),
                })
            }
            ObjectCommand::Array { command } => {
                let (object, path) = match command {
                    ObjectArrayCommand::Set {
                        object, property, ..
                    }
                    | ObjectArrayCommand::Append {
                        object, property, ..
                    }
                    | ObjectArrayCommand::Remove {
                        object, property, ..
                    }
                    | ObjectArrayCommand::Clear { object, property } => (object, property),
                };
                let object = typed_target(object);
                let (property, field) = typed_property_path(path)?;
                one(match command {
                    ObjectArrayCommand::Set { index, values, .. } => {
                        EditActionKind::ObjectArraySet {
                            object,
                            property,
                            field,
                            index: usize::from(*index),
                            values: values.clone(),
                        }
                    }
                    ObjectArrayCommand::Append { values, .. } => {
                        EditActionKind::ObjectArrayAppend {
                            object,
                            property,
                            field,
                            values: values.clone(),
                        }
                    }
                    ObjectArrayCommand::Remove { index, .. } => EditActionKind::ObjectArrayRemove {
                        object,
                        property,
                        field,
                        index: usize::from(*index),
                    },
                    ObjectArrayCommand::Clear { .. } => EditActionKind::ObjectArrayClear {
                        object,
                        property,
                        field,
                    },
                })
            }
            ObjectCommand::BytesImport { id, property, file } => {
                let bytes = read_asset(&base.join(file))
                    .map_err(|e| error("INPUT_FAILED", e.to_string()))?;
                one(EditActionKind::ObjectBytesSet {
                    object: typed_target(id),
                    property: typed_property(property)?,
                    bytes,
                })
            }
            _ => Ok(None),
        },
        Command::Id { command } => match command {
            IdCommand::Reassign { include_overrides } => one(EditActionKind::ObjectIdReassign {
                include_overrides: *include_overrides,
                mappings: None,
            }),
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
                    return Err(error("INVALID_USAGE", format!("Repeated setting '{key}'")));
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
            let (object, property, target) = match command {
                ReferenceCommand::Set {
                    object,
                    property,
                    target,
                } => (object, property, optional_target(target)),
                ReferenceCommand::Clear { object, property } => (object, property, None),
            };
            one(EditActionKind::Operation {
                operation: Operation::SetObjectReference {
                    object: typed_target(object),
                    property: typed_property(property)?,
                    reference: target,
                },
            })
        }
        Command::Pointer { command } => {
            let (object, targets) = match command {
                PointerCommand::Set { object, targets } => (object, targets.as_slice()),
                PointerCommand::Clear { object } => (object, &[][..]),
            };
            one(EditActionKind::Operation {
                operation: Operation::SetObjectPointerAllowedReferences {
                    object: typed_target(object),
                    references: targets.iter().map(|s| typed_target(s)).collect(),
                },
            })
        }
        Command::List { command } => one(EditActionKind::RelationshipEdit {
            edit: match command {
                ListCommand::Set { object, targets } => RelationshipEdit::ListSet {
                    object: typed_target(object),
                    required: None,
                    objects: targets.iter().map(|s| optional_target(s)).collect(),
                },
                ListCommand::Clear { object } => RelationshipEdit::ListSet {
                    object: typed_target(object),
                    required: None,
                    objects: Vec::new(),
                },
                ListCommand::Insert {
                    object,
                    index,
                    target,
                } => RelationshipEdit::ListInsert {
                    object: typed_target(object),
                    index: *index,
                    target: optional_target(target),
                },
                ListCommand::Remove { object, index } => RelationshipEdit::ListRemove {
                    object: typed_target(object),
                    index: *index,
                },
            },
        }),
        Command::Event { command } => one(EditActionKind::RelationshipEdit {
            edit: match command {
                EventCommand::Bind { object, bindings }
                | EventCommand::Set { object, bindings } => RelationshipEdit::MacroBindings {
                    object: typed_target(object),
                    replace: matches!(command, EventCommand::Set { .. }),
                    bindings: bindings
                        .iter()
                        .map(|binding| {
                            let (event, target) = binding
                                .split_once('=')
                                .ok_or_else(|| error("INVALID_USAGE", "expected EVENT=TARGET"))?;
                            Ok(crate::MacroReferenceTarget {
                                event_id: typed_event(event)?,
                                macro_object: typed_target(target),
                            })
                        })
                        .collect::<Result<_, Value>>()?,
                },
                EventCommand::Clear { object } => RelationshipEdit::MacroBindings {
                    object: typed_target(object),
                    replace: true,
                    bindings: Vec::new(),
                },
                EventCommand::Unbind { object, event } => RelationshipEdit::MacroUnbind {
                    object: typed_target(object),
                    event: typed_event(event)?,
                },
                EventCommand::Order { object, events } => RelationshipEdit::MacroOrder {
                    object: typed_target(object),
                    events: events
                        .iter()
                        .map(|e| typed_event(e))
                        .collect::<Result<_, _>>()?,
                },
            },
        }),
        Command::Macro { command } => match command {
            MacroCommand::Append {
                target,
                instruction,
                arguments,
            } => one(EditActionKind::MacroAppend {
                object: typed_target(target),
                instruction: instruction.clone(),
                arguments: arguments.clone(),
            }),
        },
        Command::Label { command } => one(EditActionKind::RelationshipEdit {
            edit: match command {
                LabelCommand::Clear { object } => RelationshipEdit::LabelClear {
                    object: typed_target(object),
                },
                LabelCommand::Remove { object, id } => RelationshipEdit::LabelRemove {
                    object: typed_target(object),
                    id: typed_target(id),
                },
                LabelCommand::Add { object, fields } | LabelCommand::Set { object, fields } => {
                    let mut values = BTreeMap::new();
                    for field in fields {
                        let (key, value) = field
                            .split_once('=')
                            .ok_or_else(|| error("INVALID_USAGE", "expected FIELD=VALUE"))?;
                        if !["id", "string", "graphic", "font"].contains(&key)
                            || values.insert(key, value).is_some()
                        {
                            return Err(error(
                                "INVALID_USAGE",
                                format!("Unknown or repeated label field '{key}'"),
                            ));
                        }
                    }
                    RelationshipEdit::LabelSet {
                        object: typed_target(object),
                        replace: matches!(command, LabelCommand::Set { .. }),
                        id: typed_target(
                            values.get("id").ok_or_else(|| {
                                error("INVALID_USAGE", "label requires id=TARGET")
                            })?,
                        ),
                        string: values.get("string").and_then(|s| optional_target(s)),
                        graphic: values.get("graphic").and_then(|s| optional_target(s)),
                        font: values
                            .get("font")
                            .ok_or_else(|| error("INVALID_USAGE", "label requires font=NUMBER"))?
                            .parse::<u8>()
                            .map_err(|_| error("INVALID_USAGE", "font must be 0..255"))?,
                    }
                }
            },
        }),
        Command::Component { command } => match command {
            ComponentCommand::NodeTextSet {
                component_id,
                node_id,
                property,
                text,
                encoding,
            } => author_component(
                crate::cli::component_authoring::ComponentAuthoringAction::NodePropertySet {
                    component_id: component_target(project, component_id)?,
                    node_id: node_target(project, component_id, node_id)?,
                    property: typed_property(property)?,
                    value: crate::cli::string_authoring::encode(text, encoding)
                        .map_err(|message| error("INVALID_USAGE", message))?,
                },
            ),
            ComponentCommand::Instantiate {
                component_id,
                parent,
                x,
                y,
            } => one(EditActionKind::Component {
                action: crate::components::ComponentIntent::InstantiateComponent {
                    instance_id: None,
                    component_id: component_target(project, component_id)?,
                    parent: crate::cli::names::object_id(
                        project.unwrap(),
                        parent.strip_prefix("name:").unwrap_or(parent),
                    )
                    .map_err(|message| error("INVALID_USAGE", message))?,
                    x: *x,
                    y: *y,
                    overrides: BTreeMap::new(),
                },
            }),
            ComponentCommand::Move { instance_id, x, y } => one(EditActionKind::ComponentMove {
                instance_id: instance_target(project, instance_id)?,
                x: *x,
                y: *y,
            }),
            ComponentCommand::Duplicate { instance_id, x, y } => {
                one(EditActionKind::ComponentDuplicate {
                    instance_id: instance_target(project, instance_id)?,
                    x: *x,
                    y: *y,
                })
            }
            ComponentCommand::Delete { instance_id } => one(EditActionKind::ComponentDelete {
                instance_id: instance_target(project, instance_id)?,
            }),
            ComponentCommand::Detach { instance_id } => one(EditActionKind::ComponentDetach {
                instance_id: instance_target(project, instance_id)?,
            }),
            ComponentCommand::Override {
                instance_id,
                parameter,
                value,
            } => one(EditActionKind::ComponentOverride {
                instance_id: instance_target(project, instance_id)?,
                parameter: parameter.clone(),
                value: value.clone(),
            }),
            ComponentCommand::Reset {
                instance_id,
                parameter,
            } => one(EditActionKind::ComponentReset {
                instance_id: instance_target(project, instance_id)?,
                parameter: parameter.clone(),
            }),
            ComponentCommand::DefinitionRename { component_id, name } => {
                one(EditActionKind::ComponentDefinitionRename {
                    component_id: component_target(project, component_id)?,
                    name: name.clone(),
                })
            }
            ComponentCommand::DefinitionDuplicate { component_id } => {
                one(EditActionKind::ComponentDefinitionDuplicate {
                    component_id: component_target(project, component_id)?,
                })
            }
            ComponentCommand::DefinitionDelete { component_id } => {
                one(EditActionKind::ComponentDefinitionDelete {
                    component_id: component_target(project, component_id)?,
                })
            }
            ComponentCommand::NodeSet {
                component_id,
                node_id,
                property,
                value,
            } => one(EditActionKind::ComponentNodeSet {
                component_id: component_target(project, component_id)?,
                node_id: node_target(project, component_id, node_id)?,
                property: typed_property(property)?,
                value: value.clone(),
            }),
            ComponentCommand::NodeReference {
                component_id,
                node_id,
                property,
                target,
            } => one(EditActionKind::ComponentNodeReference {
                component_id: component_target(project, component_id)?,
                node_id: node_target(project, component_id, node_id)?,
                property: typed_property(property)?,
                target: target.clone(),
            }),
            ComponentCommand::ParameterRemove {
                component_id,
                parameter_id,
            } => one(EditActionKind::ComponentParameterRemove {
                component_id: component_target(project, component_id)?,
                parameter_id: parameter_target(project, component_id, parameter_id)?,
            }),
            ComponentCommand::Capture { root, name } => one(EditActionKind::ComponentCapture {
                root: crate::cli::names::object_id(
                    project.unwrap(),
                    root.strip_prefix("name:").unwrap_or(root),
                )
                .map_err(|message| error("INVALID_USAGE", message))?,
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
                component_id: component_target(project, component_id)?,
                parameter: ParameterInput {
                    id: None,
                    name: name.clone(),
                    description: None,
                    target_node: node_target(project, component_id, target_node)?,
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
                component_id: component_target(project, component_id)?,
                parameter: ParameterInput {
                    id: Some(parameter_target(project, component_id, parameter_id)?),
                    name: name.clone(),
                    description: None,
                    target_node: node_target(project, component_id, target_node)?,
                    property: typed_parameter_target(property)?,
                    kind: typed_parameter_kind(kind)?,
                    default_value: Some(default_value.clone()),
                },
            }),
            ComponentCommand::DefinitionCreate {
                name,
                root_name,
                root_type,
                description,
                category_path,
                tags,
            } => author_component(
                crate::cli::component_authoring::ComponentAuthoringAction::DefinitionCreate {
                    id: crate::components::ComponentId(uuid::Uuid::new_v4()),
                    name: name.clone(),
                    description: description.clone(),
                    category_path: category_path.clone(),
                    tags: tags.clone(),
                    revision: 0,
                    root_node: crate::components::ComponentNodeId(uuid::Uuid::new_v4()),
                    root_name: root_name.clone(),
                    root_type: typed_object_type(root_type)?,
                },
            ),
            ComponentCommand::DefinitionMetadata {
                component_id,
                name,
                description,
                category_path,
                tags,
                revision,
            } => author_component(
                crate::cli::component_authoring::ComponentAuthoringAction::DefinitionMetadata {
                    component_id: component_target(project, component_id)?,
                    name: name.clone(),
                    description: description.clone(),
                    category_path: category_path.clone(),
                    tags: tags.clone(),
                    revision: *revision,
                },
            ),
            ComponentCommand::NodeCreate {
                component_id,
                name,
                object_type,
            } => author_component(
                crate::cli::component_authoring::ComponentAuthoringAction::NodeCreate {
                    component_id: component_target(project, component_id)?,
                    id: crate::components::ComponentNodeId(uuid::Uuid::new_v4()),
                    name: name.clone(),
                    object_type: typed_object_type(object_type)?,
                },
            ),
            ComponentCommand::NodeOrderSet {
                component_id,
                nodes,
            } => author_component(
                crate::cli::component_authoring::ComponentAuthoringAction::NodeOrderSet {
                    component_id: component_target(project, component_id)?,
                    nodes: nodes
                        .iter()
                        .map(|name| node_target(project, component_id, name))
                        .collect::<Result<_, _>>()?,
                },
            ),
            ComponentCommand::NodeRename {
                component_id,
                node_id,
                name,
            } => author_component(
                crate::cli::component_authoring::ComponentAuthoringAction::NodeMetadata {
                    component_id: component_target(project, component_id)?,
                    node_id: node_target(project, component_id, node_id)?,
                    name: name.clone(),
                },
            ),
            ComponentCommand::NodePropertySet {
                component_id,
                node_id,
                property,
                value,
            } => author_component(
                crate::cli::component_authoring::ComponentAuthoringAction::NodePropertySet {
                    component_id: component_target(project, component_id)?,
                    node_id: node_target(project, component_id, node_id)?,
                    property: typed_property(property)?,
                    value: parse_typed_json(value, "component property value")?,
                },
            ),
            ComponentCommand::NodeAssignFile {
                component_id,
                node_id,
                file,
                format,
                encoding,
            } => {
                let path = base.join(file);
                let bytes = read_asset(&path).map_err(|e| error("INPUT_FAILED", e))?;
                let format = match format.as_str() {
                    "monochrome" => PictureGraphicFormat::Monochrome,
                    "four_bit" => PictureGraphicFormat::FourBit,
                    "eight_bit" => PictureGraphicFormat::EightBit,
                    _ => unreachable!("clap validates picture graphic format"),
                };
                let encoding = match encoding.as_str() {
                    "raw" => DataCodeType::Raw,
                    "run_length" => DataCodeType::RunLength,
                    _ => unreachable!("clap validates picture graphic encoding"),
                };
                author_component(
                    crate::cli::component_authoring::ComponentAuthoringAction::NodePictureAssignFile {
                        component_id: component_target(project, component_id)?,
                        node_id: node_target(project, component_id, node_id)?,
                        bytes,
                        format,
                        encoding,
                    },
                )
            }
            ComponentCommand::NodeReferenceSet {
                component_id,
                node_id,
                property,
                target,
            } => author_component(
                crate::cli::component_authoring::ComponentAuthoringAction::NodeReferenceSet {
                    component_id: component_target(project, component_id)?,
                    node_id: node_target(project, component_id, node_id)?,
                    property: typed_property(property)?,
                    target: parse_component_reference(project, component_id, target)?,
                },
            ),
            ComponentCommand::NodeMacroAppend {
                component_id,
                node_id,
                instruction,
                arguments,
            } => author_component(
                crate::cli::component_authoring::ComponentAuthoringAction::NodeMacroAppend {
                    component_id: component_target(project, component_id)?,
                    node_id: node_target(project, component_id, node_id)?,
                    instruction: instruction.clone(),
                    arguments: crate::cli::editing::resolve_macro_object_arguments(
                        project.unwrap(),
                        instruction,
                        arguments,
                    )
                    .map_err(|message| error("INVALID_USAGE", message))?,
                },
            ),
            ComponentCommand::NodeMacroClear {
                component_id,
                node_id,
            } => author_component(
                crate::cli::component_authoring::ComponentAuthoringAction::NodeMacroClear {
                    component_id: component_target(project, component_id)?,
                    node_id: node_target(project, component_id, node_id)?,
                },
            ),
            ComponentCommand::NodeChildrenSet {
                component_id,
                node_id,
                children,
            } => {
                let children = children
                    .iter()
                    .map(|spec| {
                        let mut parts = spec.rsplitn(3, ':');
                        let y = parts.next().ok_or_else(|| {
                            error("INVALID_USAGE", "child requires NODE_NAME:X:Y")
                        })?;
                        let x = parts.next().ok_or_else(|| {
                            error("INVALID_USAGE", "child requires NODE_NAME:X:Y")
                        })?;
                        let child = parts.next().ok_or_else(|| {
                            error("INVALID_USAGE", "child requires NODE_NAME:X:Y")
                        })?;
                        Ok(crate::components::ComponentChild {
                            child: node_target(project, component_id, child)?,
                            x: x.parse().map_err(|_| {
                                error("INVALID_USAGE", "child x must be an integer")
                            })?,
                            y: y.parse().map_err(|_| {
                                error("INVALID_USAGE", "child y must be an integer")
                            })?,
                        })
                    })
                    .collect::<Result<Vec<_>, Value>>()?;
                author_component(
                    crate::cli::component_authoring::ComponentAuthoringAction::NodeChildrenSet {
                        component_id: component_target(project, component_id)?,
                        node_id: node_target(project, component_id, node_id)?,
                        children,
                    },
                )
            }
            ComponentCommand::NodeMacroReferencesSet {
                component_id,
                node_id,
                references,
            } => author_component(
                crate::cli::component_authoring::ComponentAuthoringAction::NodeMacroReferencesSet {
                    component_id: component_target(project, component_id)?,
                    node_id: node_target(project, component_id, node_id)?,
                    references: parse_named_macro_references(references)?,
                },
            ),
            ComponentCommand::NodeObjectListSet {
                component_id,
                node_id,
                object_list,
            } => author_component(
                crate::cli::component_authoring::ComponentAuthoringAction::NodeObjectListSet {
                    component_id: component_target(project, component_id)?,
                    node_id: node_target(project, component_id, node_id)?,
                    object_list: parse_named_object_list(object_list)?,
                },
            ),
            ComponentCommand::ParameterAddTyped {
                component_id,
                name,
                target_node,
                property,
                kind,
                default_value,
                description,
            } => {
                let kind = typed_parameter_kind(kind)?;
                author_component(
                    crate::cli::component_authoring::ComponentAuthoringAction::ParameterAdd {
                        component_id: component_target(project, component_id)?,
                        parameter: crate::components::ComponentParameter {
                            id: crate::components::ComponentParameterId(uuid::Uuid::new_v4()),
                            name: name.clone(),
                            description: description.clone(),
                            target_node: node_target(project, component_id, target_node)?,
                            property: typed_parameter_target(property)?,
                            kind,
                            default_value: parse_parameter_default(project, kind, default_value)?,
                        },
                    },
                )
            }
            ComponentCommand::ParameterSetTyped {
                component_id,
                parameter,
                name,
                target_node,
                property,
                kind,
                default_value,
                description,
            } => {
                let kind = typed_parameter_kind(kind)?;
                author_component(
                    crate::cli::component_authoring::ComponentAuthoringAction::ParameterSet {
                        component_id: component_target(project, component_id)?,
                        parameter: crate::components::ComponentParameter {
                            id: parameter_target(project, component_id, parameter)?,
                            name: name.clone(),
                            description: description.clone(),
                            target_node: node_target(project, component_id, target_node)?,
                            property: typed_parameter_target(property)?,
                            kind,
                            default_value: parse_parameter_default(project, kind, default_value)?,
                        },
                    },
                )
            }
            ComponentCommand::InstanceCreate {
                name,
                component_id,
                parent,
                x,
                y,
                child_index,
                overrides,
                compiled_ids,
            } => {
                let mut compiled = BTreeMap::new();
                for mapping in compiled_ids {
                    let (node, object) = mapping.rsplit_once('=').ok_or_else(|| {
                        error("INVALID_USAGE", "compiled override requires NODE_NAME=ID")
                    })?;
                    if compiled
                        .insert(node_target(project, component_id, node)?, cli_id(object)?)
                        .is_some()
                    {
                        return Err(error("INVALID_USAGE", "repeated compiled node name"));
                    }
                }
                author_component(
                    crate::cli::component_authoring::ComponentAuthoringAction::InstanceCreate {
                        id: crate::components::ComponentInstanceId(uuid::Uuid::new_v4()),
                        name: name.clone(),
                        component_id: component_target(project, component_id)?,
                        parent: crate::cli::names::object_id(
                            project.unwrap(),
                            parent.strip_prefix("name:").unwrap_or(parent),
                        )
                        .map_err(|message| error("INVALID_USAGE", message))?,
                        x: *x,
                        y: *y,
                        child_index: *child_index,
                        overrides: parse_named_component_overrides(
                            project,
                            component_id,
                            overrides,
                        )?,
                        compiled_ids: compiled,
                    },
                )
            }
            ComponentCommand::InstanceMetadata {
                instance_id,
                child_index,
                compiled_ids,
            } => {
                let mut compiled = BTreeMap::new();
                for mapping in compiled_ids {
                    let (node, object) = mapping.rsplit_once('=').ok_or_else(|| {
                        error("INVALID_USAGE", "compiled override requires NODE_NAME=ID")
                    })?;
                    if compiled
                        .insert(
                            crate::cli::names::node_id_for_instance(
                                project.unwrap(),
                                instance_id,
                                node,
                            )
                            .map_err(|message| error("INVALID_USAGE", message))?,
                            cli_id(object)?,
                        )
                        .is_some()
                    {
                        return Err(error("INVALID_USAGE", "repeated compiled node name"));
                    }
                }
                author_component(
                    crate::cli::component_authoring::ComponentAuthoringAction::InstanceMetadata {
                        instance_id: instance_target(project, instance_id)?,
                        child_index: *child_index,
                        compiled_ids: compiled,
                    },
                )
            }
            ComponentCommand::OverrideSet {
                instance_id,
                parameter_id,
                value,
            } => {
                let project_ref = project.ok_or_else(|| {
                    error(
                        "INVALID_USAGE",
                        "Component override requires a loaded project",
                    )
                })?;
                let resolved_parameter = crate::cli::names::parameter_id_for_instance(
                    project_ref,
                    instance_id,
                    parameter_id,
                )
                .map_err(|message| error("INVALID_USAGE", message))?;
                let instance = project_ref
                    .component_source()
                    .instances
                    .iter()
                    .find(|instance| crate::smart_naming::names_equal(&instance.name, instance_id))
                    .ok_or_else(|| {
                        error(
                            "INVALID_USAGE",
                            format!("Unknown component instance '{instance_id}'"),
                        )
                    })?;
                let parsed: Value = parse_typed_json(value, "component override")?;
                let component_name = project_ref.component_source().definitions
                    [&instance.component_id]
                    .name
                    .clone();
                let resolved = resolve_named_parameter_value(
                    Some(project_ref),
                    &component_name,
                    resolved_parameter,
                    parsed,
                )?;
                author_component(
                    crate::cli::component_authoring::ComponentAuthoringAction::OverrideSet {
                        instance_id: instance.id,
                        parameter_id: resolved_parameter,
                        value: resolved,
                    },
                )
            }
            _ => Ok(None),
        },
        Command::Graphic { command } => match command {
            GraphicsCommand::Import {
                file,
                name,
                description,
                tags,
                original_name,
                clear_original_name,
            } => {
                let path = base.join(file);
                let bytes = read_asset(&path).map_err(|e| error("INPUT_FAILED", e))?;
                one(EditActionKind::GraphicImport {
                    filename: path
                        .file_name()
                        .and_then(|s| s.to_str())
                        .unwrap_or("graphic")
                        .to_owned(),
                    bytes,
                    name: name.clone(),
                    asset_id: None,
                    original_file_name: original_name.clone(),
                    clear_original_name: *clear_original_name,
                    description: description.clone(),
                    tags: tags.clone(),
                })
            }
            GraphicsCommand::Assign { asset_id, object } => one(EditActionKind::GraphicAssign {
                asset_id: graphic_target(project, asset_id)?,
                object: crate::cli::names::object_id(
                    project.unwrap(),
                    object.strip_prefix("name:").unwrap_or(object),
                )
                .map_err(|message| error("INVALID_USAGE", message))?,
            }),
            GraphicsCommand::AssignFile {
                object,
                file,
                format,
                encoding,
            } => {
                let path = base.join(file);
                let bytes = read_asset(&path).map_err(|e| error("INPUT_FAILED", e))?;
                let format = match format.as_str() {
                    "monochrome" => PictureGraphicFormat::Monochrome,
                    "four_bit" => PictureGraphicFormat::FourBit,
                    "eight_bit" => PictureGraphicFormat::EightBit,
                    _ => unreachable!("clap validates picture graphic format"),
                };
                let encoding = match encoding.as_str() {
                    "raw" => DataCodeType::Raw,
                    "run_length" => DataCodeType::RunLength,
                    _ => unreachable!("clap validates picture graphic encoding"),
                };
                one(EditActionKind::GraphicAssignFile {
                    object: crate::cli::names::object_id(
                        project.unwrap(),
                        object.strip_prefix("name:").unwrap_or(object),
                    )
                    .map_err(|message| error("INVALID_USAGE", message))?,
                    bytes,
                    format,
                    encoding,
                })
            }
            GraphicsCommand::CreatePicture { asset_id } => {
                one(EditActionKind::GraphicCreatePicture {
                    asset_id: graphic_target(project, asset_id)?,
                })
            }
            GraphicsCommand::Rename { asset_id, name } => one(EditActionKind::GraphicRename {
                asset_id: graphic_target(project, asset_id)?,
                name: name.clone(),
            }),
            GraphicsCommand::Delete { asset_id } => one(EditActionKind::GraphicDelete {
                asset_id: graphic_target(project, asset_id)?,
            }),
            _ => Ok(None),
        },
        _ => Ok(None),
    }
}

fn error(code: &str, message: impl Into<String>) -> Value {
    json!({"error":{"code":code,"message":message.into()}})
}

/// Shell-style quoting without expansion or command execution.
pub fn split_line(line: &str) -> Result<Vec<String>, String> {
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

/// Quote one literal command token without shell expansion. Literal newlines are
/// retained inside quotes and are understood by the command file parser.
pub fn quote_token(token: &str) -> String {
    if !token.is_empty()
        && token
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || b"_-./:=,@+".contains(&byte))
    {
        token.to_owned()
    } else {
        format!("'{}'", token.replace('\'', "'\\''"))
    }
}

/// Serialize literal arguments into one readable command.
pub fn serialize_command<S: AsRef<str>>(tokens: &[S]) -> String {
    tokens
        .iter()
        .map(|token| quote_token(token.as_ref()))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Parse a command file through the same command tree and edit translation as
/// direct CLI authoring. Assets resolve relative to `base` using `read_asset`,
/// allowing both native files and browser ZIP assets. The whole file is atomic
/// and accepts at most [`MAX_EDIT_ACTIONS`] edits.
pub fn parse_commands(
    contents: &str,
    base: &Path,
    read_asset: &dyn Fn(&Path) -> Result<Vec<u8>, String>,
) -> Result<Vec<EditAction>, Value> {
    let mut logical = Vec::new();
    let mut pending = String::new();
    let mut quote = None;
    let mut escaped = false;
    let mut comment = false;
    let mut line = 1usize;
    let mut start = 1usize;
    for c in contents.chars() {
        if comment {
            if c == '\n' {
                comment = false;
                line += 1;
                start = line;
            }
            continue;
        }
        if escaped {
            if c == '\n' {
                pending.pop();
                line += 1;
            } else {
                pending.push(c);
            }
            escaped = false;
            continue;
        }
        if quote == Some('\'') {
            pending.push(c);
            if c == '\'' {
                quote = None;
            }
        } else if c == '\\' {
            pending.push(c);
            escaped = true;
        } else if let Some(q) = quote {
            pending.push(c);
            if c == q {
                quote = None;
            }
        } else if c == '\'' || c == '"' {
            pending.push(c);
            quote = Some(c);
        } else if c == '#' && pending.trim().is_empty() {
            pending.clear();
            comment = true;
        } else if c == '\n' {
            logical.push((start, std::mem::take(&mut pending)));
            line += 1;
            start = line;
            continue;
        } else {
            pending.push(c);
        }
        if c == '\n' {
            line += 1;
        }
    }
    if escaped || quote.is_some() {
        return Err(error(
            "SCRIPT_PARSE_ERROR",
            format!("line {start}: unterminated quote or escape"),
        ));
    }
    if !pending.is_empty() {
        logical.push((start, pending));
    }
    let mut actions = Vec::new();
    for (line_number, line) in logical {
        let tokens = split_line(&line)
            .map_err(|e| error("SCRIPT_PARSE_ERROR", format!("line {line_number}: {e}")))?;
        if tokens.is_empty() {
            continue;
        }
        let cli =
            Cli::try_parse_from(std::iter::once("aitd").chain(tokens.iter().map(String::as_str)))
                .map_err(|e| error("SCRIPT_PARSE_ERROR", format!("line {line_number}: {e}")))?;
        if cli.json || cli.check {
            return Err(error("SCRIPT_COMMAND_UNSUPPORTED", format!("line {line_number}: select output format and --check on run, not inside the batch")));
        }
        let edits = command_edit_actions(&cli.command, base, read_asset)
            .map_err(|mut e| {
                if let Some(message) = e.pointer_mut("/error/message") {
                    if let Some(text) = message.as_str().map(str::to_owned) {
                        *message = json!(format!("line {line_number}: {text}"));
                    }
                }
                e
            })?
            .ok_or_else(|| {
                error(
                    "SCRIPT_COMMAND_UNSUPPORTED",
                    format!(
                        "line {line_number}: output or inspection commands cannot run in a batch"
                    ),
                )
            })?;
        actions.extend(edits.into_iter().map(|mut action| {
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
    Ok(actions)
}
