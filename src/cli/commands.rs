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

#[derive(Debug, Subcommand, Clone)]
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
        root: u16,
        #[arg(long, default_value_t = 6)]
        depth: usize,
    },
    /// Read a design context.
    Context {
        #[arg(allow_hyphen_values = true, long, default_value = "active_mask")]
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
        #[arg(allow_hyphen_values = true, long, value_delimiter = ',')]
        properties: Vec<String>,
    },
    /// Validate the current project.
    Validate,
    /// Render a preview to a PNG file.
    Preview {
        #[arg(long)]
        mask: Option<u16>,
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
        root: u16,
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
#[derive(Debug, Subcommand, Clone)]
pub enum BackupCommand {
    List {
        #[arg(allow_hyphen_values = true, long)]
        project: Option<PathBuf>,
    },
}
#[derive(Debug, Subcommand, Clone)]
pub enum ObjectCommand {
    /// Set readable text with an explicit VT character encoding.
    TextSet {
        #[arg(allow_hyphen_values = true)]
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
        #[arg(allow_hyphen_values = true)]
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
        #[arg(
            allow_hyphen_values = true,
            required_unless_present = "named",
            conflicts_with = "named"
        )]
        name: Option<String>,
        #[arg(allow_hyphen_values = true, long = "name")]
        named: Option<String>,
        /// Assign an explicit object ID.
        #[arg(long)]
        id: Option<u16>,
    },
    Set {
        #[arg(allow_hyphen_values = true)]
        id: String,
        #[arg(required = true)]
        assignments: Vec<String>,
    },
    Rename {
        #[arg(allow_hyphen_values = true)]
        id: String,
        #[arg(allow_hyphen_values = true)]
        name: String,
    },
    /// Set editor metadata without changing pool properties.
    Metadata {
        #[arg(allow_hyphen_values = true)]
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
        /// Whether the current object ID is fixed or assigned automatically.
        #[arg(allow_hyphen_values = true, long, value_parser = ["fixed", "automatic"])]
        id_policy: Option<String>,
    },
    Delete {
        #[arg(allow_hyphen_values = true)]
        id: String,
    },
    Order {
        #[arg(allow_hyphen_values = true)]
        ids: Vec<String>,
    },
    Duplicate {
        id: u16,
        #[arg(allow_hyphen_values = true, long)]
        name: Option<String>,
    },
    Field {
        #[arg(allow_hyphen_values = true)]
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
        #[arg(allow_hyphen_values = true)]
        id: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
        #[arg(allow_hyphen_values = true)]
        file: PathBuf,
    },
    /// Save a byte-valued property without JSON encoding.
    BytesExport {
        #[arg(allow_hyphen_values = true)]
        id: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
        #[arg(allow_hyphen_values = true, long)]
        output: PathBuf,
        #[arg(long)]
        force: bool,
    },
}
#[derive(Debug, Subcommand, Clone)]
pub enum ObjectArrayCommand {
    Set {
        #[arg(allow_hyphen_values = true)]
        object: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
        index: u16,
        #[arg(allow_hyphen_values = true, required = true)]
        values: Vec<String>,
    },
    Append {
        #[arg(allow_hyphen_values = true)]
        object: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
        #[arg(allow_hyphen_values = true, required = true)]
        values: Vec<String>,
    },
    Remove {
        #[arg(allow_hyphen_values = true)]
        object: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
        index: u16,
    },
    Clear {
        #[arg(allow_hyphen_values = true)]
        object: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
    },
}
#[derive(Debug, Subcommand, Clone)]
pub enum ComponentCommand {
    /// Set readable node text with an explicit VT character encoding.
    NodeTextSet {
        #[arg(allow_hyphen_values = true)]
        component_id: String,
        #[arg(allow_hyphen_values = true)]
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
        #[arg(allow_hyphen_values = true)]
        id: String,
        #[arg(long)]
        nodes: bool,
    },
    Instantiate {
        #[arg(allow_hyphen_values = true)]
        component_id: String,
        parent: u16,
        #[arg(allow_negative_numbers = true)]
        x: i32,
        #[arg(allow_negative_numbers = true)]
        y: i32,
    },
    Move {
        #[arg(allow_hyphen_values = true)]
        instance_id: String,
        #[arg(allow_negative_numbers = true)]
        x: i32,
        #[arg(allow_negative_numbers = true)]
        y: i32,
    },
    Duplicate {
        #[arg(allow_hyphen_values = true)]
        instance_id: String,
        #[arg(allow_negative_numbers = true)]
        x: i32,
        #[arg(allow_negative_numbers = true)]
        y: i32,
    },
    Delete {
        #[arg(allow_hyphen_values = true)]
        instance_id: String,
    },
    Detach {
        #[arg(allow_hyphen_values = true)]
        instance_id: String,
    },
    Override {
        #[arg(allow_hyphen_values = true)]
        instance_id: String,
        #[arg(allow_hyphen_values = true)]
        parameter: String,
        #[arg(allow_hyphen_values = true)]
        value: String,
    },
    Reset {
        #[arg(allow_hyphen_values = true)]
        instance_id: String,
        #[arg(allow_hyphen_values = true)]
        parameter: String,
    },
    DefinitionRename {
        #[arg(allow_hyphen_values = true)]
        component_id: String,
        #[arg(allow_hyphen_values = true)]
        name: String,
    },
    DefinitionDuplicate {
        #[arg(allow_hyphen_values = true)]
        component_id: String,
    },
    DefinitionDelete {
        #[arg(allow_hyphen_values = true)]
        component_id: String,
    },
    NodeSet {
        #[arg(allow_hyphen_values = true)]
        component_id: String,
        #[arg(allow_hyphen_values = true)]
        node_id: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
        #[arg(allow_hyphen_values = true)]
        value: String,
    },
    /// Assign a node reference to an object, node:UUID, or null.
    NodeReference {
        #[arg(allow_hyphen_values = true)]
        component_id: String,
        #[arg(allow_hyphen_values = true)]
        node_id: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
        #[arg(allow_hyphen_values = true)]
        target: String,
    },
    ParameterRemove {
        #[arg(allow_hyphen_values = true)]
        component_id: String,
        #[arg(allow_hyphen_values = true)]
        parameter_id: String,
    },
    Capture {
        root: u16,
        #[arg(allow_hyphen_values = true)]
        name: String,
    },
    ParameterAdd {
        #[arg(allow_hyphen_values = true)]
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
        #[arg(allow_hyphen_values = true)]
        component_id: String,
        #[arg(allow_hyphen_values = true)]
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
        id: String,
        #[arg(allow_hyphen_values = true)]
        name: String,
        #[arg(allow_hyphen_values = true)]
        root_node: String,
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
        #[arg(allow_hyphen_values = true)]
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
        #[arg(allow_hyphen_values = true)]
        component_id: String,
        #[arg(allow_hyphen_values = true)]
        node_id: String,
        #[arg(allow_hyphen_values = true)]
        name: String,
        #[arg(allow_hyphen_values = true)]
        object_type: String,
    },
    NodeOrderSet {
        #[arg(allow_hyphen_values = true)]
        component_id: String,
        #[arg(allow_hyphen_values = true, required = true)]
        nodes: Vec<String>,
    },
    NodeRename {
        #[arg(allow_hyphen_values = true)]
        component_id: String,
        #[arg(allow_hyphen_values = true)]
        node_id: String,
        #[arg(allow_hyphen_values = true)]
        name: String,
    },
    NodePropertySet {
        #[arg(allow_hyphen_values = true)]
        component_id: String,
        #[arg(allow_hyphen_values = true)]
        node_id: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
        /// A typed JSON value such as `"Hello"`, `true`, or `17`.
        #[arg(allow_hyphen_values = true)]
        value: String,
    },
    /// Assign indexed PNG image data to a PictureGraphic component node.
    NodeAssignFile {
        #[arg(allow_hyphen_values = true)]
        component_id: String,
        #[arg(allow_hyphen_values = true)]
        node_id: String,
        file: PathBuf,
        #[arg(long, value_parser = ["monochrome", "four_bit", "eight_bit"])]
        format: String,
        #[arg(long, value_parser = ["raw", "run_length"])]
        encoding: String,
    },
    /// Append one readable instruction to a component Macro node.
    NodeMacroAppend {
        #[arg(allow_hyphen_values = true)]
        component_id: String,
        #[arg(allow_hyphen_values = true)]
        node_id: String,
        #[arg(allow_hyphen_values = true)]
        instruction: String,
        #[arg(allow_hyphen_values = true, required = true)]
        arguments: Vec<String>,
    },
    /// Set a component Macro node's Commands property to an empty stream.
    NodeMacroClear {
        #[arg(allow_hyphen_values = true)]
        component_id: String,
        #[arg(allow_hyphen_values = true)]
        node_id: String,
    },
    NodeReferenceSet {
        #[arg(allow_hyphen_values = true)]
        component_id: String,
        #[arg(allow_hyphen_values = true)]
        node_id: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
        /// A typed ComponentReference JSON value.
        #[arg(allow_hyphen_values = true)]
        target: String,
    },
    NodeChildrenSet {
        #[arg(allow_hyphen_values = true)]
        component_id: String,
        #[arg(allow_hyphen_values = true)]
        node_id: String,
        /// Ordered child specifications `NODE_UUID:X:Y`.
        #[arg(allow_hyphen_values = true, required = true)]
        children: Vec<String>,
    },
    NodeMacroReferencesSet {
        #[arg(allow_hyphen_values = true)]
        component_id: String,
        #[arg(allow_hyphen_values = true)]
        node_id: String,
        /// Typed JSON array of `{ "event": ..., "target": ... }` records.
        #[arg(allow_hyphen_values = true)]
        references: String,
    },
    NodeObjectListSet {
        #[arg(allow_hyphen_values = true)]
        component_id: String,
        #[arg(allow_hyphen_values = true)]
        node_id: String,
        /// Typed ComponentObjectList JSON, or `null` to clear.
        #[arg(allow_hyphen_values = true)]
        object_list: String,
    },
    ParameterAddTyped {
        #[arg(allow_hyphen_values = true)]
        component_id: String,
        #[arg(allow_hyphen_values = true)]
        id: String,
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
        #[arg(allow_hyphen_values = true)]
        component_id: String,
        #[arg(allow_hyphen_values = true)]
        id: String,
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
        id: String,
        #[arg(allow_hyphen_values = true)]
        component_id: String,
        parent: u16,
        #[arg(allow_negative_numbers = true)]
        x: i32,
        #[arg(allow_negative_numbers = true)]
        y: i32,
        #[arg(long)]
        child_index: Option<usize>,
        #[arg(allow_hyphen_values = true, long, default_value = "{}")]
        overrides: String,
        #[arg(allow_hyphen_values = true, long = "compiled-id")]
        compiled_ids: Vec<String>,
    },
    InstanceMetadata {
        #[arg(allow_hyphen_values = true)]
        instance_id: String,
        #[arg(long)]
        child_index: Option<usize>,
        #[arg(allow_hyphen_values = true, long = "compiled-id")]
        compiled_ids: Vec<String>,
    },
    OverrideSet {
        #[arg(allow_hyphen_values = true)]
        instance_id: String,
        #[arg(allow_hyphen_values = true)]
        parameter_id: String,
        #[arg(allow_hyphen_values = true)]
        value: String,
    },
}
#[derive(Debug, Subcommand, Clone)]
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
#[derive(Debug, Subcommand, Clone)]
pub enum ProjectSettingsCommand {
    Show,
    /// Change settings with typed key=value assignments.
    Set {
        #[arg(required = true)]
        assignments: Vec<String>,
    },
}
#[derive(Debug, Subcommand, Clone)]
pub enum InstanceCommand {
    List,
    Get {
        #[arg(allow_hyphen_values = true)]
        id: String,
    },
}
#[derive(Debug, Subcommand, Clone)]
pub enum PlacementCommand {
    Add {
        #[arg(allow_hyphen_values = true)]
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
        #[arg(allow_hyphen_values = true)]
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
        #[arg(allow_hyphen_values = true)]
        parent: String,
        #[arg(allow_hyphen_values = true)]
        child: String,
        #[arg(long)]
        occurrence: Option<usize>,
    },
    Order {
        #[arg(allow_hyphen_values = true)]
        parent: String,
        #[arg(allow_hyphen_values = true)]
        children: Vec<String>,
    },
}
#[derive(Debug, Subcommand, Clone)]
pub enum GraphicsCommand {
    Get {
        #[arg(allow_hyphen_values = true)]
        id: String,
    },
    /// Save an available asset or PictureGraphic as a PNG.
    Export {
        #[arg(allow_hyphen_values = true)]
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
        /// Stable UUID to use for this available graphic.
        #[arg(allow_hyphen_values = true, long)]
        id: Option<String>,
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
        #[arg(allow_hyphen_values = true)]
        asset_id: String,
        #[arg(allow_hyphen_values = true)]
        object: String,
    },
    /// Assign an indexed PNG directly, retaining palette indices and requested encoding.
    AssignFile {
        #[arg(allow_hyphen_values = true)]
        object: String,
        #[arg(allow_hyphen_values = true)]
        file: PathBuf,
        #[arg(allow_hyphen_values = true, long, value_parser = ["monochrome", "four_bit", "eight_bit"])]
        format: String,
        #[arg(allow_hyphen_values = true, long, value_parser = ["raw", "run_length"])]
        encoding: String,
    },
    CreatePicture {
        #[arg(allow_hyphen_values = true)]
        asset_id: String,
    },
    Rename {
        #[arg(allow_hyphen_values = true)]
        asset_id: String,
        #[arg(allow_hyphen_values = true)]
        name: String,
    },
    Delete {
        #[arg(allow_hyphen_values = true)]
        asset_id: String,
    },
}
#[derive(Debug, Subcommand, Clone)]
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

#[derive(Debug, Subcommand, Clone)]
pub enum IdCommand {
    Set {
        #[arg(allow_hyphen_values = true)]
        object: String,
        #[arg(allow_hyphen_values = true)]
        id: String,
    },
    Remap {
        #[arg(required = true)]
        mappings: Vec<String>,
    },
    Auto {
        #[arg(allow_hyphen_values = true)]
        object: String,
    },
    Reassign {
        #[arg(long)]
        include_overrides: bool,
    },
}
#[derive(Debug, Subcommand, Clone)]
pub enum ReferenceCommand {
    Set {
        #[arg(allow_hyphen_values = true)]
        object: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
        #[arg(allow_hyphen_values = true)]
        target: String,
    },
    Clear {
        #[arg(allow_hyphen_values = true)]
        object: String,
        #[arg(allow_hyphen_values = true)]
        property: String,
    },
}
#[derive(Debug, Subcommand, Clone)]
pub enum PointerCommand {
    Set {
        #[arg(allow_hyphen_values = true)]
        object: String,
        #[arg(allow_hyphen_values = true)]
        targets: Vec<String>,
    },
    Clear {
        #[arg(allow_hyphen_values = true)]
        object: String,
    },
}
#[derive(Debug, Subcommand, Clone)]
pub enum ListCommand {
    Set {
        #[arg(allow_hyphen_values = true)]
        object: String,
        #[arg(allow_hyphen_values = true)]
        targets: Vec<String>,
    },
    Insert {
        #[arg(allow_hyphen_values = true)]
        object: String,
        index: usize,
        #[arg(allow_hyphen_values = true)]
        target: String,
    },
    Remove {
        #[arg(allow_hyphen_values = true)]
        object: String,
        index: usize,
    },
    Clear {
        #[arg(allow_hyphen_values = true)]
        object: String,
    },
}
#[derive(Debug, Subcommand, Clone)]
pub enum EventCommand {
    Bind {
        #[arg(allow_hyphen_values = true)]
        object: String,
        #[arg(required = true)]
        bindings: Vec<String>,
    },
    Set {
        #[arg(allow_hyphen_values = true)]
        object: String,
        #[arg(required = true)]
        bindings: Vec<String>,
    },
    Unbind {
        #[arg(allow_hyphen_values = true)]
        object: String,
        #[arg(allow_hyphen_values = true)]
        event: String,
    },
    Order {
        #[arg(allow_hyphen_values = true)]
        object: String,
        #[arg(required = true)]
        events: Vec<String>,
    },
    Clear {
        #[arg(allow_hyphen_values = true)]
        object: String,
    },
}

#[derive(Debug, Subcommand, Clone)]
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
#[derive(Debug, Subcommand, Clone)]
pub enum LabelCommand {
    Add {
        #[arg(allow_hyphen_values = true)]
        object: String,
        #[arg(required = true)]
        fields: Vec<String>,
    },
    Set {
        #[arg(allow_hyphen_values = true)]
        object: String,
        #[arg(required = true)]
        fields: Vec<String>,
    },
    Remove {
        #[arg(allow_hyphen_values = true)]
        object: String,
        #[arg(allow_hyphen_values = true)]
        id: String,
    },
    Clear {
        #[arg(allow_hyphen_values = true)]
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

/// Build a CLI command that completely reproduces an object's editor metadata.
pub fn object_metadata_tokens(id: u16, info: &crate::ObjectInfo) -> Vec<String> {
    let mut tokens = vec!["object".into(), "metadata".into(), id.to_string()];
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
    tokens.extend([
        "--id-policy".into(),
        if info.id_overridden {
            "fixed"
        } else {
            "automatic"
        }
        .into(),
    ]);
    tokens
}

fn append_optional_metadata(tokens: &mut Vec<String>, field: &str, value: Option<&str>) {
    if let Some(value) = value {
        tokens.extend([format!("--{field}"), value.to_owned()]);
    } else {
        tokens.push(format!("--clear-{field}"));
    }
}

/// Build a CLI command for one decoded Macro instruction.
pub fn macro_instruction_tokens(
    target: u16,
    instruction: &crate::macro_authoring::MacroInstruction,
) -> Vec<String> {
    let mut tokens = vec![
        "macro".into(),
        "append".into(),
        target.to_string(),
        instruction.name.clone(),
    ];
    tokens.extend(instruction.arguments.iter().cloned());
    tokens
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
                named,
                id,
            } => one(EditActionKind::Operation {
                operation: Operation::CreateObject {
                    object_id: *id,
                    object_type: typed_object_type(object_type)?,
                    name: name.as_ref().or(named.as_ref()).unwrap().clone(),
                },
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
            ObjectCommand::Rename { id, name } => one(EditActionKind::Operation {
                operation: Operation::RenameObject {
                    object: typed_target(id),
                    name: name.clone(),
                },
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
                id_policy,
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
                id_overridden: id_policy.as_deref().map(|policy| policy == "fixed"),
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
            IdCommand::Set { object, id } => one(EditActionKind::Operation {
                operation: Operation::SetObjectIdAssignment {
                    object_id: cli_id(object)?,
                    new_id: cli_id(id)?,
                    overridden: true,
                },
            }),
            IdCommand::Auto { object } => one(EditActionKind::ObjectIdAuto {
                object_id: cli_id(object)?,
            }),
            IdCommand::Reassign { include_overrides } => one(EditActionKind::ObjectIdReassign {
                include_overrides: *include_overrides,
                mappings: None,
            }),
            IdCommand::Remap { mappings } => one(EditActionKind::Operation {
                operation: Operation::RemapObjectIds {
                    mappings: mappings
                        .iter()
                        .map(|pair| {
                            let (old, new) = pair.split_once(':').ok_or_else(|| {
                                error("INVALID_USAGE", "expected old:new ID pair")
                            })?;
                            Ok((cli_id(old)?, cli_id(new)?))
                        })
                        .collect::<Result<_, Value>>()?,
                    target_types: Vec::new(),
                },
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
                    component_id: typed_id(component_id, "component ID")?,
                    node_id: typed_id(node_id, "component node ID")?,
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
            ComponentCommand::DefinitionCreate {
                id,
                name,
                root_node,
                root_name,
                root_type,
                description,
                category_path,
                tags,
            } => author_component(
                crate::cli::component_authoring::ComponentAuthoringAction::DefinitionCreate {
                    id: typed_id(id, "component ID")?,
                    name: name.clone(),
                    description: description.clone(),
                    category_path: category_path.clone(),
                    tags: tags.clone(),
                    revision: 0,
                    root_node: typed_id(root_node, "component node ID")?,
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
                    component_id: typed_id(component_id, "component ID")?,
                    name: name.clone(),
                    description: description.clone(),
                    category_path: category_path.clone(),
                    tags: tags.clone(),
                    revision: *revision,
                },
            ),
            ComponentCommand::NodeCreate {
                component_id,
                node_id,
                name,
                object_type,
            } => author_component(
                crate::cli::component_authoring::ComponentAuthoringAction::NodeCreate {
                    component_id: typed_id(component_id, "component ID")?,
                    id: typed_id(node_id, "component node ID")?,
                    name: name.clone(),
                    object_type: typed_object_type(object_type)?,
                },
            ),
            ComponentCommand::NodeOrderSet {
                component_id,
                nodes,
            } => author_component(
                crate::cli::component_authoring::ComponentAuthoringAction::NodeOrderSet {
                    component_id: typed_id(component_id, "component ID")?,
                    nodes: nodes
                        .iter()
                        .map(|id| typed_id(id, "component node ID"))
                        .collect::<Result<_, _>>()?,
                },
            ),
            ComponentCommand::NodeRename {
                component_id,
                node_id,
                name,
            } => author_component(
                crate::cli::component_authoring::ComponentAuthoringAction::NodeMetadata {
                    component_id: typed_id(component_id, "component ID")?,
                    node_id: typed_id(node_id, "component node ID")?,
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
                    component_id: typed_id(component_id, "component ID")?,
                    node_id: typed_id(node_id, "component node ID")?,
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
                        component_id: typed_id(component_id, "component ID")?,
                        node_id: typed_id(node_id, "component node ID")?,
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
                    component_id: typed_id(component_id, "component ID")?,
                    node_id: typed_id(node_id, "component node ID")?,
                    property: typed_property(property)?,
                    target: parse_typed_json(target, "component reference")?,
                },
            ),
            ComponentCommand::NodeMacroAppend {
                component_id,
                node_id,
                instruction,
                arguments,
            } => author_component(
                crate::cli::component_authoring::ComponentAuthoringAction::NodeMacroAppend {
                    component_id: typed_id(component_id, "component ID")?,
                    node_id: typed_id(node_id, "component node ID")?,
                    instruction: instruction.clone(),
                    arguments: arguments.clone(),
                },
            ),
            ComponentCommand::NodeMacroClear {
                component_id,
                node_id,
            } => author_component(
                crate::cli::component_authoring::ComponentAuthoringAction::NodeMacroClear {
                    component_id: typed_id(component_id, "component ID")?,
                    node_id: typed_id(node_id, "component node ID")?,
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
                        let mut parts = spec.split(':');
                        let child = parts.next().ok_or_else(|| {
                            error("INVALID_USAGE", "child requires NODE_UUID:X:Y")
                        })?;
                        let x = parts.next().ok_or_else(|| {
                            error("INVALID_USAGE", "child requires NODE_UUID:X:Y")
                        })?;
                        let y = parts.next().ok_or_else(|| {
                            error("INVALID_USAGE", "child requires NODE_UUID:X:Y")
                        })?;
                        if parts.next().is_some() {
                            return Err(error("INVALID_USAGE", "child requires NODE_UUID:X:Y"));
                        }
                        Ok(crate::components::ComponentChild {
                            child: typed_id(child, "component node ID")?,
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
                        component_id: typed_id(component_id, "component ID")?,
                        node_id: typed_id(node_id, "component node ID")?,
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
                    component_id: typed_id(component_id, "component ID")?,
                    node_id: typed_id(node_id, "component node ID")?,
                    references: parse_typed_json(references, "component macro references")?,
                },
            ),
            ComponentCommand::NodeObjectListSet {
                component_id,
                node_id,
                object_list,
            } => author_component(
                crate::cli::component_authoring::ComponentAuthoringAction::NodeObjectListSet {
                    component_id: typed_id(component_id, "component ID")?,
                    node_id: typed_id(node_id, "component node ID")?,
                    object_list: parse_typed_json(object_list, "component object list")?,
                },
            ),
            ComponentCommand::ParameterAddTyped {
                component_id,
                id,
                name,
                target_node,
                property,
                kind,
                default_value,
                description,
            } => author_component(
                crate::cli::component_authoring::ComponentAuthoringAction::ParameterAdd {
                    component_id: typed_id(component_id, "component ID")?,
                    parameter: crate::components::ComponentParameter {
                        id: typed_id(id, "component parameter ID")?,
                        name: name.clone(),
                        description: description.clone(),
                        target_node: typed_id(target_node, "component node ID")?,
                        property: typed_parameter_target(property)?,
                        kind: typed_parameter_kind(kind)?,
                        default_value: parse_typed_json(default_value, "parameter default")?,
                    },
                },
            ),
            ComponentCommand::ParameterSetTyped {
                component_id,
                id,
                name,
                target_node,
                property,
                kind,
                default_value,
                description,
            } => author_component(
                crate::cli::component_authoring::ComponentAuthoringAction::ParameterSet {
                    component_id: typed_id(component_id, "component ID")?,
                    parameter: crate::components::ComponentParameter {
                        id: typed_id(id, "component parameter ID")?,
                        name: name.clone(),
                        description: description.clone(),
                        target_node: typed_id(target_node, "component node ID")?,
                        property: typed_parameter_target(property)?,
                        kind: typed_parameter_kind(kind)?,
                        default_value: parse_typed_json(default_value, "parameter default")?,
                    },
                },
            ),
            ComponentCommand::InstanceCreate {
                id,
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
                    let (node, object) = mapping.split_once('=').ok_or_else(|| {
                        error("INVALID_USAGE", "compiled ID requires NODE_UUID=OBJECT_ID")
                    })?;
                    if compiled
                        .insert(typed_id(node, "component node ID")?, cli_id(object)?)
                        .is_some()
                    {
                        return Err(error("INVALID_USAGE", "repeated compiled node ID"));
                    }
                }
                author_component(
                    crate::cli::component_authoring::ComponentAuthoringAction::InstanceCreate {
                        id: typed_id(id, "component instance ID")?,
                        component_id: typed_id(component_id, "component ID")?,
                        parent: *parent,
                        x: *x,
                        y: *y,
                        child_index: *child_index,
                        overrides: parse_typed_json(overrides, "component overrides")?,
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
                    let (node, object) = mapping.split_once('=').ok_or_else(|| {
                        error("INVALID_USAGE", "compiled ID requires NODE_UUID=OBJECT_ID")
                    })?;
                    if compiled
                        .insert(typed_id(node, "component node ID")?, cli_id(object)?)
                        .is_some()
                    {
                        return Err(error("INVALID_USAGE", "repeated compiled node ID"));
                    }
                }
                author_component(
                    crate::cli::component_authoring::ComponentAuthoringAction::InstanceMetadata {
                        instance_id: typed_id(instance_id, "component instance ID")?,
                        child_index: *child_index,
                        compiled_ids: compiled,
                    },
                )
            }
            ComponentCommand::OverrideSet {
                instance_id,
                parameter_id,
                value,
            } => author_component(
                crate::cli::component_authoring::ComponentAuthoringAction::OverrideSet {
                    instance_id: typed_id(instance_id, "component instance ID")?,
                    parameter_id: typed_id(parameter_id, "component parameter ID")?,
                    value: parse_typed_json(value, "component override")?,
                },
            ),
            _ => Ok(None),
        },
        Command::Graphic { command } => match command {
            GraphicsCommand::Import {
                file,
                id,
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
                    asset_id: id
                        .as_deref()
                        .map(|id| typed_id(id, "graphic asset ID"))
                        .transpose()?,
                    original_file_name: original_name.clone(),
                    clear_original_name: *clear_original_name,
                    description: description.clone(),
                    tags: tags.clone(),
                })
            }
            GraphicsCommand::Assign { asset_id, object } => one(EditActionKind::GraphicAssign {
                asset_id: typed_id(asset_id, "graphic asset ID")?,
                object: cli_id(object)?,
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
                    object: cli_id(object)?,
                    bytes,
                    format,
                    encoding,
                })
            }
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
    format!("'{}'", token.replace('\'', "'\\''"))
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

#[cfg(test)]
mod command_tests {
    use super::*;
    #[test]
    fn serialized_tokens_preserve_difficult_text() {
        let tokens = [
            "object",
            "set",
            "Text",
            "value=quote ' slash \\ newline\n\t tab\r\n$HOME `test`",
            "",
        ];
        assert_eq!(split_line(&serialize_command(&tokens)).unwrap(), tokens);
        let edits = parse_commands(
            &serialize_command(&tokens[..4]),
            Path::new("."),
            &|_| unreachable!(),
        )
        .unwrap();
        match &edits[0].kind {
            EditActionKind::SetProperties { values, .. } => assert_eq!(
                values[&PropertyKey::Value],
                tokens[3].strip_prefix("value=").unwrap()
            ),
            _ => panic!("wrong command"),
        }
    }
    #[test]
    fn signed_positions_and_hyphen_leading_text_are_literal_arguments() {
        for tokens in [
            vec![
                "object",
                "create",
                "NumberVariable",
                "--foo",
                "--id",
                "6100",
            ],
            vec![
                "object",
                "create",
                "NumberVariable",
                "--name",
                "-test",
                "--id",
                "6100",
            ],
            vec![
                "object",
                "metadata",
                "6100",
                "--name",
                "--foo",
                "--description",
                "-test",
                "--notes",
                "--foo",
                "--tag",
                "-test",
            ],
            vec!["placement", "add", "1000", "6100", "-3", "-12"],
            vec!["placement", "order", "1000", "--foo", "-test"],
            vec![
                "placement",
                "move",
                "1000",
                "6100",
                "-32768",
                "32767",
                "--occurrence",
                "0",
            ],
        ] {
            let text = serialize_command(&tokens);
            assert_eq!(
                parse_commands(&text, Path::new("."), &|_| unreachable!())
                    .unwrap()
                    .len(),
                1,
                "{text}"
            );
        }
        let cli = Cli::try_parse_from([
            "aitd",
            "object",
            "create",
            "NumberVariable",
            "--foo",
            "--id",
            "6100",
        ])
        .unwrap();
        match cli.command {
            Command::Object {
                command: ObjectCommand::Create { name, id, .. },
            } => {
                assert_eq!(name.as_deref(), Some("--foo"));
                assert_eq!(id, Some(6100));
            }
            _ => panic!("wrong command"),
        }
    }
    #[test]
    fn parses_comments_continuations_and_reports_unfinished_strings() {
        let text = "# comment's ignored\nobject set Text \\\n 'value=hello'\n";
        assert_eq!(
            parse_commands(text, Path::new("."), &|_| unreachable!())
                .unwrap()
                .len(),
            1
        );
        assert!(parse_commands(
            "object set Text 'value=unfinished",
            Path::new("."),
            &|_| unreachable!()
        )
        .is_err());
    }
}
