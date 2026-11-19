//! Per-object-type automatic ID allocation ranges.

use ag_iso_stack::object_pool::{vt_version::VtVersion, ObjectType};
use serde::{Deserialize, Serialize};
use std::fmt;

/// Inclusive range used for automatic object ID assignment.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ObjectIdRange {
    pub start: u16,
    pub end: u16,
}

/// Configurable automatic ID ranges for every object type supported by the stack.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ObjectIdRanges(Vec<ObjectIdRangeEntry>);

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct ObjectIdRangeEntry {
    object_type: ObjectType,
    range: ObjectIdRange,
}

impl ObjectIdRanges {
    /// Default ranges: Working Set 256–999, Macro 0–255, and each other type
    /// gets its 1000-ID block based on its ISO object type number.
    pub fn defaults() -> Self {
        let ranges = ObjectType::values()
            .into_iter()
            .map(|object_type| {
                let object_number = u16::from(u8::from(object_type));
                let range = match object_type {
                    ObjectType::WorkingSet => ObjectIdRange {
                        start: 256,
                        end: 999,
                    },
                    ObjectType::Macro => ObjectIdRange { start: 0, end: 255 },
                    _ => {
                        let start = object_number * 1000;
                        ObjectIdRange {
                            start,
                            end: start + 999,
                        }
                    }
                };
                ObjectIdRangeEntry { object_type, range }
            })
            .collect();
        Self(ranges)
    }

    /// Return the configured range for `object_type`.
    pub fn range_for(&self, object_type: ObjectType) -> ObjectIdRange {
        self.0
            .iter()
            .find(|entry| entry.object_type == object_type)
            .map(|entry| entry.range)
            .unwrap_or_else(|| Self::defaults().range_for(object_type))
    }

    /// Set one type's range while preserving all other ranges.
    pub fn set_range(&mut self, object_type: ObjectType, range: ObjectIdRange) {
        if let Some(entry) = self
            .0
            .iter_mut()
            .find(|entry| entry.object_type == object_type)
        {
            entry.range = range;
        } else {
            self.0.push(ObjectIdRangeEntry { object_type, range });
        }
    }

    /// Iterate configured ranges in object type order.
    pub fn iter(&self) -> impl Iterator<Item = (ObjectType, ObjectIdRange)> + '_ {
        self.0.iter().map(|entry| (entry.object_type, entry.range))
    }

    /// Validate complete type coverage, bounds, overlap, and macro limits for
    /// the selected VT target.
    pub fn validate(&self, version: VtVersion) -> Result<(), String> {
        let supported = ObjectType::values();
        if self.0.len() != supported.len()
            || supported.iter().any(|object_type| {
                self.0
                    .iter()
                    .filter(|entry| entry.object_type == *object_type)
                    .count()
                    != 1
            })
        {
            return Err(
                "Configure exactly one automatic ID range for every supported object type".into(),
            );
        }

        let mut occupied = Vec::with_capacity(self.0.len());
        for entry in &self.0 {
            let range = entry.range;
            if range.start > range.end {
                return Err(format!(
                    "Invalid {:?} ID range: start exceeds end",
                    entry.object_type
                ));
            }
            if range.end == u16::MAX {
                return Err(format!(
                    "{:?} ID range must stay within 0–65534",
                    entry.object_type
                ));
            }
            if entry.object_type == ObjectType::Macro && u8::from(version) <= 4 && range.end > 255 {
                return Err("VT2–VT4 macro ID ranges must stay within 0–255".into());
            }
            occupied.push((range.start, range.end, entry.object_type));
        }
        occupied.sort_by_key(|(start, end, _)| (*start, *end));
        for pair in occupied.windows(2) {
            if pair[1].0 <= pair[0].1 {
                return Err(format!(
                    "Automatic ID ranges for {:?} and {:?} overlap",
                    pair[0].2, pair[1].2
                ));
            }
        }
        Ok(())
    }
}

impl Default for ObjectIdRanges {
    fn default() -> Self {
        Self::defaults()
    }
}

impl fmt::Display for ObjectIdRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}–{}", self.start, self.end)
    }
}
