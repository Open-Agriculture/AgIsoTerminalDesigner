//! ISO 11783-6 Table A.4: 8-bit colour indices, not arbitrary RGB values.
//! RGB values reuse AgIsoStack's palette, also used by the pool renderer.
//! Reference: ISO 11783-6:2018, A.3 / Table A.4.
use ag_iso_stack::object_pool::Colour;
use serde_json::{json, Value};

pub const BASIC_NAMES: [&str; 16] = [
    "Black", "White", "Green", "Teal", "Maroon", "Purple", "Olive", "Silver", "Grey", "Blue",
    "Lime", "Cyan", "Red", "Magenta", "Yellow", "Navy",
];
pub const DEFAULT_ALPHA: u8 = 255;
pub const FIRST_PROPRIETARY: u8 = 232;
pub const CUBE_LEVELS: [u8; 6] = [0, 51, 102, 153, 204, 255];

/// Proprietary indices have no standard RGB value.
pub fn standard_rgb(index: u8) -> Option<[u8; 3]> {
    (index < FIRST_PROPRIETARY).then(|| Colour::COLOUR_PALETTE[index as usize].as_rgb())
}

pub fn label(index: u8) -> String {
    if let Some(name) = BASIC_NAMES.get(index as usize) {
        format!("{index}: {name}")
    } else if let Some([r, g, b]) = standard_rgb(index) {
        format!("{index}: #{r:02X}{g:02X}{b:02X}")
    } else {
        format!("{index}: Manufacturer-specific")
    }
}

/// Compact, complete palette definition for AI object schemas.
pub fn schema() -> Value {
    json!({
        "standard": "ISO 11783-6:2018 A.3 / Table A.4",
        "default_alpha": DEFAULT_ALPHA,
        "colour_modes": {"monochrome": [0, 1], "16_colour": [0, 15], "256_colour": [0, 255]},
        "scope": "Standard palette. Active Colour Map or Colour Palette objects may override appearance.",
        "encoding": "8-bit integer index (0..255). Set Colour properties to the index, not RGB or a name.",
        "basic_colours": BASIC_NAMES.iter().enumerate().map(|(index, name)| {
            json!({"index": index, "name": name, "rgb": standard_rgb(index as u8)})
        }).collect::<Vec<_>>(),
        "colour_cube": {
            "index_range": [16, FIRST_PROPRIETARY - 1],
            "rgb_levels": CUBE_LEVELS,
            "index_formula": "16 + 36*r + 6*g + b; r, g, b are level indices 0..5"
        },
        "manufacturer_specific": {
            "index_range": [FIRST_PROPRIETARY, 255],
            "meaning": "No standard RGB value; only use with a known terminal-specific definition."
        }
    })
}
