//! Application-level presentation and input for ISO object identifiers.

use serde::{Deserialize, Serialize};

/// Object-ID radix used by the editor UI; never changes stored IDs or exports.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ObjectIdFormat {
    #[default]
    Decimal,
    Hexadecimal,
}

impl ObjectIdFormat {
    /// Format an ID, including the null-reference sentinel when appropriate.
    pub fn format(self, id: u16) -> String {
        match self {
            Self::Decimal => id.to_string(),
            Self::Hexadecimal => format!("0x{id:04X}"),
        }
    }

    /// Parse an unsigned 16-bit ID. An explicit `0x`/`0X` prefix always uses
    /// hexadecimal; otherwise the selected radix applies. Object existence,
    /// uniqueness and null-reference validity are checked by the caller.
    pub fn parse(self, text: &str) -> Result<u16, std::num::ParseIntError> {
        let text = text.trim();
        let (digits, radix) = match text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")) {
            Some(digits) => (digits, 16),
            None => (text, if self == Self::Hexadecimal { 16 } else { 10 }),
        };
        u16::from_str_radix(digits, radix)
    }
}
