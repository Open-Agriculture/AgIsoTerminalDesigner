//! Copyright 2024 - The Open-Agriculture Developers
//! SPDX-License-Identifier: GPL-3.0-or-later
//! Authors: Daan Steenbergen

use ag_iso_stack::object_pool::ObjectPool;
use std::time::SystemTime;

/// Represents a change to the object pool with metadata for history tracking
#[derive(Clone)]
pub struct Change {
    /// Human-readable description of what changed
    pub description: String,

    /// The state of the object pool after this change was applied
    pub pool_state: ObjectPool,

    /// When this change was made
    pub timestamp: SystemTime,

    /// Optional category for grouping/filtering changes
    pub category: ChangeCategory,
}

/// Categories of changes for better organization and display
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ChangeCategory {
    /// A new object was added
    ObjectAdded,

    /// An object was deleted
    ObjectDeleted,

    /// An object's properties were modified
    ObjectModified,

    /// An object was renamed
    ObjectRenamed,

    /// Multiple objects were changed at once
    BulkChange,

    /// Initial state or other uncategorized change
    Other,
}

impl Change {
    /// Create a new change with a description and the resulting pool state
    pub fn new(description: String, pool_state: ObjectPool, category: ChangeCategory) -> Self {
        Self {
            description,
            pool_state,
            timestamp: SystemTime::now(),
            category,
        }
    }

    /// Get a formatted timestamp string for display
    pub fn formatted_time(&self) -> String {
        if let Ok(duration) = self.timestamp.elapsed() {
            let secs = duration.as_secs();
            if secs < 60 {
                format!("{}s ago", secs)
            } else if secs < 3600 {
                format!("{}m ago", secs / 60)
            } else if secs < 86400 {
                format!("{}h ago", secs / 3600)
            } else {
                format!("{}d ago", secs / 86400)
            }
        } else {
            "just now".to_string()
        }
    }

    /// Get an icon/emoji for the change category
    pub fn category_icon(&self) -> &str {
        match self.category {
            ChangeCategory::ObjectAdded => "➕",
            ChangeCategory::ObjectDeleted => "🗑",
            ChangeCategory::ObjectModified => "✏",
            ChangeCategory::ObjectRenamed => "📝",
            ChangeCategory::BulkChange => "📦",
            ChangeCategory::Other => "•",
        }
    }

    /// Get a color hint for the change category (as RGB)
    pub fn category_color(&self) -> [u8; 3] {
        match self.category {
            ChangeCategory::ObjectAdded => [0, 200, 0],      // Green
            ChangeCategory::ObjectDeleted => [200, 0, 0],    // Red
            ChangeCategory::ObjectModified => [0, 100, 200], // Blue
            ChangeCategory::ObjectRenamed => [150, 100, 200], // Purple
            ChangeCategory::BulkChange => [200, 150, 0],     // Orange
            ChangeCategory::Other => [128, 128, 128],        // Gray
        }
    }
}
