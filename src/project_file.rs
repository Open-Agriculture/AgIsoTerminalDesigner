//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

use crate::{ObjectIdRanges, ObjectInfo, SoftKeyLayout};
use ag_iso_stack::object_pool::{object::Object, vt_version::VtVersion, ObjectId, ObjectPool};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};

/// Project file format version
const PROJECT_FILE_VERSION: u32 = 2;

/// AgIsoTerminalProject file format (.aitp)
/// This format stores both the object pool and custom metadata
#[derive(Serialize, Deserialize)]
pub struct ProjectFile {
    /// Version of the project file format
    version: u32,

    #[serde(default)]
    pub components: crate::components::ComponentSource,
    #[serde(default)]
    pub graphics: crate::graphics::GraphicLibrary,

    /// Historical v1 codec bytes, independent of the saved target version.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    object_pool_data: Vec<u8>,

    /// Version-neutral v2 authoring objects, retaining invalid repair drafts.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    objects: Option<Vec<Object>>,

    /// Custom metadata for objects (names, etc.)
    object_metadata: BTreeMap<u16, ObjectMetadata>,

    /// Project-level settings
    settings: ProjectSettings,
}

/// Metadata for a single object
#[derive(Serialize, Deserialize, Clone)]
pub struct ObjectMetadata {
    /// Custom name for the object
    pub name: Option<String>,

    /// Notes or comments about the object
    #[serde(default)]
    pub notes: Option<String>,

    #[serde(default)]
    pub description: Option<String>,

    #[serde(default)]
    pub tags: Vec<String>,

    /// Ordered editor-only ObjectPointer targets; empty preserves unrestricted selection.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_references: Vec<ObjectId>,

    /// Whether this object keeps an explicitly assigned ID.
    #[serde(default)]
    pub id_overridden: bool,
}

/// Project-level settings
#[derive(Serialize, Deserialize, Clone)]
pub struct ProjectSettings {
    /// Virtual mask size for preview
    pub mask_size: u16,

    /// Softkey designator width and height in design pixels. Missing legacy
    /// values are inferred from Key contents when opening the project.
    #[serde(default)]
    pub soft_key_size: Option<(u16, u16)>,

    /// Stored since project format v1; missing values in legacy files use VT6.
    #[serde(default = "default_vt_version_number")]
    pub vt_version: u8,

    /// Optional preview arrangement for referenced Soft Key Masks.
    #[serde(default)]
    pub soft_key_layout: SoftKeyLayout,

    /// Last selected object ID
    pub last_selected: Option<u16>,

    /// Inclusive per-type automatic ID allocation ranges. Missing in legacy
    /// project files, which receive the current defaults without ID changes.
    #[serde(default)]
    pub object_id_ranges: ObjectIdRanges,
}

impl ProjectFile {
    /// Create a new project file from an ObjectPool and metadata.
    /// Authoring content is retained without imposing target IOP restrictions.
    pub fn new(
        pool: &ObjectPool,
        object_info: &HashMap<ObjectId, ObjectInfo>,
        mask_size: u16,
        soft_key_size: (u16, u16),
        vt_version: VtVersion,
        selected: Option<ObjectId>,
    ) -> Result<Self, serde_json::Error> {
        Self::new_with_soft_key_layout(
            pool,
            object_info,
            mask_size,
            soft_key_size,
            SoftKeyLayout::default(),
            vt_version,
            selected,
        )
    }

    /// Create a project file with an explicit preview softkey layout.
    pub fn new_with_soft_key_layout(
        pool: &ObjectPool,
        object_info: &HashMap<ObjectId, ObjectInfo>,
        mask_size: u16,
        soft_key_size: (u16, u16),
        soft_key_layout: SoftKeyLayout,
        vt_version: VtVersion,
        selected: Option<ObjectId>,
    ) -> Result<Self, serde_json::Error> {
        Self::new_with_id_ranges(
            pool,
            object_info,
            mask_size,
            soft_key_size,
            soft_key_layout,
            vt_version,
            selected,
            ObjectIdRanges::defaults(),
        )
    }

    /// Create a project file with the complete authoring settings.
    pub fn new_with_id_ranges(
        pool: &ObjectPool,
        object_info: &HashMap<ObjectId, ObjectInfo>,
        mask_size: u16,
        soft_key_size: (u16, u16),
        soft_key_layout: SoftKeyLayout,
        vt_version: VtVersion,
        selected: Option<ObjectId>,
        object_id_ranges: ObjectIdRanges,
    ) -> Result<Self, serde_json::Error> {
        object_id_ranges.validate(vt_version).map_err(|message| {
            serde_json::Error::io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                message,
            ))
        })?;
        // Convert ObjectInfo map to ObjectMetadata map
        let mut object_metadata = BTreeMap::new();
        for (id, info) in object_info {
            let metadata = ObjectMetadata {
                name: info.name.clone(),
                notes: info.notes.clone(),
                description: info.description.clone(),
                tags: info.tags.clone(),
                allowed_references: info.allowed_references.clone(),
                id_overridden: info.id_overridden,
            };
            object_metadata.insert(id.value(), metadata);
        }

        Ok(ProjectFile {
            version: PROJECT_FILE_VERSION,
            components: Default::default(),
            graphics: Default::default(),
            object_pool_data: Vec::new(),
            objects: Some(pool.objects().to_vec()),
            object_metadata,
            settings: ProjectSettings {
                mask_size,
                soft_key_size: Some(soft_key_size),
                vt_version: vt_version.into(),
                soft_key_layout,
                last_selected: selected.map(|id| id.value()),
                object_id_ranges,
            },
        })
    }

    /// Load object pool from project file
    /// Returns an error if the object pool data is corrupted or invalid
    pub fn load_pool(&self) -> Result<ObjectPool, String> {
        if self.version == 1 {
            return ObjectPool::try_from_legacy_project_iop(self.object_pool_data.clone())
                .map_err(|error| error.to_string());
        }
        let objects = self
            .objects
            .as_ref()
            .ok_or("Version 2 project is missing objects")?;
        let mut pool = ObjectPool::new();
        for object in objects {
            if pool.object_by_id(object.id()).is_some() {
                return Err(format!(
                    "Duplicate project object ID {}",
                    object.id().value()
                ));
            }
            pool.add(object.clone());
        }
        Ok(pool)
    }

    /// Get object metadata
    pub fn get_metadata(&self) -> &BTreeMap<u16, ObjectMetadata> {
        &self.object_metadata
    }

    /// Get project settings
    pub fn get_settings(&self) -> &ProjectSettings {
        &self.settings
    }

    /// Serialize project to JSON bytes
    pub fn to_bytes(&self) -> Result<Vec<u8>, serde_json::Error> {
        serde_json::to_vec_pretty(self)
    }

    /// Deserialize project from JSON bytes
    pub fn from_bytes(bytes: &[u8]) -> Result<Self, serde_json::Error> {
        let project: Self = serde_json::from_slice(bytes)?;
        if !(1..=PROJECT_FILE_VERSION).contains(&project.version) {
            return Err(serde_json::Error::io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("Unsupported project format version {}", project.version),
            )));
        }
        if !(2..=6).contains(&project.settings.vt_version) {
            return Err(serde_json::Error::io(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "Unsupported target VT{}; supported targets are VT2–VT6",
                    project.settings.vt_version
                ),
            )));
        }
        project
            .settings
            .soft_key_layout
            .validate()
            .map_err(|message| {
                serde_json::Error::io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    message,
                ))
            })?;
        project
            .settings
            .object_id_ranges
            .validate(project.settings.vt_version())
            .map_err(|message| {
                serde_json::Error::io(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    message,
                ))
            })?;
        Ok(project)
    }
}

impl Default for ProjectSettings {
    fn default() -> Self {
        ProjectSettings {
            mask_size: 500,
            soft_key_size: Some((60, 60)),
            vt_version: default_vt_version_number(),
            soft_key_layout: SoftKeyLayout::default(),
            last_selected: None,
            object_id_ranges: ObjectIdRanges::defaults(),
        }
    }
}

fn default_vt_version_number() -> u8 {
    u8::from(VtVersion::Version6)
}

impl ProjectSettings {
    /// Missing legacy values default to VT6; loaded values are checked in `from_bytes`.
    pub fn vt_version(&self) -> VtVersion {
        VtVersion::try_from(self.vt_version).unwrap_or(VtVersion::Version6)
    }
}
