//! Soft key mask preview layout settings.

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
/// Placement of the softkey area relative to the data/alarm mask.
pub enum SoftKeyArrangement {
    #[default]
    Right,
    Bottom,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct SoftKeyLayout {
    pub arrangement: SoftKeyArrangement,
    pub physical_key_count: u8,
    /// Keys per column on the right, or per row below the mask.
    pub keys_per_line: u8,
    /// Zero, one or two end slots, reserved only when paging is needed.
    pub navigation_key_count: u8,
}

impl Default for SoftKeyLayout {
    fn default() -> Self {
        Self {
            arrangement: SoftKeyArrangement::Right,
            physical_key_count: 12,
            keys_per_line: 6,
            navigation_key_count: 2,
        }
    }
}

impl SoftKeyLayout {
    /// Check counts and arrangement capacity; invalid settings cannot be applied.
    pub fn validate(self) -> Result<(), String> {
        if !(1..=64).contains(&self.physical_key_count) {
            return Err("Physical key count must be between 1 and 64".into());
        }
        if !(1..=self.physical_key_count).contains(&self.keys_per_line) {
            return Err("Keys per column/row must be between 1 and the physical key count".into());
        }
        if self.navigation_key_count > 2 || self.navigation_key_count >= self.physical_key_count {
            return Err(
                "Navigation keys must be 0, 1 or 2 and fewer than the physical keys".into(),
            );
        }
        Ok(())
    }
}
