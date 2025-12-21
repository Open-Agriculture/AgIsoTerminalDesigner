//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

use crate::ObjectInfo;
use ag_iso_stack::object_pool::{object::Object, ObjectId, ObjectPool};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Project file format version
const PROJECT_FILE_VERSION: u32 = 1;

/// AgIsoTerminalProject file format (.aitp)
/// This format stores both the object pool and custom metadata
#[derive(Serialize, Deserialize)]
pub struct ProjectFile {
    /// Version of the project file format
    version: u32,

    /// The object pool data as IOP bytes
    object_pool_data: Vec<u8>,

    /// Custom metadata for objects (names, etc.)
    object_metadata: HashMap<u16, ObjectMetadata>,

    /// Project-level settings
    settings: ProjectSettings,
}

/// Metadata for a single object
#[derive(Serialize, Deserialize, Clone)]
pub struct ObjectMetadata {
    /// Custom name for the object
    pub name: Option<String>,

    /// Notes or comments about the object
    pub notes: Option<String>,
}

/// Project-level settings
#[derive(Serialize, Deserialize, Clone)]
pub struct ProjectSettings {
    /// Virtual mask size for preview
    pub mask_size: u16,

    /// Last selected object ID
    pub last_selected: Option<u16>,
}

impl ProjectFile {
    /// Create a new project file from an ObjectPool and metadata
    pub fn new(
        pool: &ObjectPool,
        object_info: &HashMap<ObjectId, ObjectInfo>,
        mask_size: u16,
        selected: Option<ObjectId>,
    ) -> Self {
        // Convert ObjectInfo map to ObjectMetadata map
        let mut object_metadata = HashMap::new();
        for (id, info) in object_info {
            let metadata = ObjectMetadata {
                name: info.name.clone(),
                notes: None, // Future feature
            };
            object_metadata.insert(id.value(), metadata);
        }

        ProjectFile {
            version: PROJECT_FILE_VERSION,
            object_pool_data: pool.as_iop(),
            object_metadata,
            settings: ProjectSettings {
                mask_size,
                last_selected: selected.map(|id| id.value()),
            },
        }
    }

    /// Load object pool from project file
    /// Returns an error if the object pool data is corrupted or invalid
    pub fn load_pool(&self) -> Result<ObjectPool, String> {
        // Validate minimum size for a valid IOP file
        if self.object_pool_data.len() < 4 {
            return Err("Object pool data is too small to be valid".to_string());
        }

        // Try to parse the object pool
        let pool = ObjectPool::from_iop(self.object_pool_data.clone());

        // Validate that we got a non-empty pool
        // Note: This is a heuristic check since from_iop might return an empty pool for invalid data
        if pool.objects().is_empty() && self.object_pool_data.len() > 4 {
            return Err("Failed to parse object pool: no objects found in data".to_string());
        }

        Ok(pool)
    }

    /// Get object metadata
    pub fn get_metadata(&self) -> &HashMap<u16, ObjectMetadata> {
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
        serde_json::from_slice(bytes)
    }
}

impl Default for ProjectSettings {
    fn default() -> Self {
        ProjectSettings {
            mask_size: 500,
            last_selected: None,
        }
    }
}
