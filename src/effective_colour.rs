//! Authoritative editor rendering palette. Only explicitly referenced palette/map
//! objects are active; orphan palette objects never change the display. If malformed
//! pools have several special-controls objects, lowest object ID wins deterministically.
use ag_iso_stack::object_pool::{object::Object, Colour, ObjectPool};
use serde::{Deserialize, Serialize};

pub fn colour(pool: &ObjectPool, index: u8) -> Colour {
    let controls = pool
        .objects()
        .iter()
        .filter_map(|o| match o {
            Object::WorkingSetSpecialControls(c) => Some(c),
            _ => None,
        })
        .min_by_key(|c| c.id.value());
    let Some(c) = controls else {
        return pool.color_by_index(index);
    };
    let mapped = c
        .id_of_colour_map
        .0
        .and_then(|id| pool.object_by_id(id))
        .and_then(|o| match o {
            Object::ColourMap(m) => m.colour_map.get(index as usize).copied(),
            _ => None,
        })
        .unwrap_or(index);
    c.id_of_colour_palette
        .0
        .and_then(|id| pool.object_by_id(id))
        .and_then(|o| match o {
            Object::ColourPalette(p) => p.colours.get(mapped as usize).copied(),
            _ => None,
        })
        .unwrap_or_else(|| pool.color_by_index(mapped))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedColour {
    pub index: u8,
    pub name: String,
    pub rgb: String,
}
pub fn resolve_index(pool: &ObjectPool, index: u8) -> ResolvedColour {
    let rgb = colour(pool, index).as_rgb();
    let name = (0..16u8)
        .find(|i| Colour::COLOUR_PALETTE[*i as usize].as_rgb() == rgb)
        .map(|i| canonical_name(i))
        .unwrap_or_else(|| format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]));
    ResolvedColour {
        index,
        name,
        rgb: format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]),
    }
}
fn canonical_name(i: u8) -> String {
    match i {
        7 => "light_gray".into(),
        8 => "gray".into(),
        15 => "dark_blue".into(),
        _ => crate::isobus_colour::BASIC_NAMES[i as usize].to_ascii_lowercase(),
    }
}
pub fn named_rgb(name: &str) -> Option<[u8; 3]> {
    let n = name.trim().to_ascii_lowercase().replace([' ', '-'], "_");
    let n = match n.as_str() {
        "navy" | "dark_navy" | "deep_blue" => "dark_blue",
        "aqua" => "cyan",
        "grey" => "gray",
        "silver" | "light_grey" => "light_gray",
        _ => &n,
    };
    (0..16)
        .find(|i| canonical_name(*i) == n)
        .map(|i| Colour::COLOUR_PALETTE[i as usize].as_rgb())
}
/// Squared Euclidean RGB distance, with lowest index as stable tie breaker.
pub fn nearest(pool: &ObjectPool, rgb: [u8; 3]) -> ResolvedColour {
    resolve_index(
        pool,
        (0..=255u8)
            .min_by_key(|i| distance(rgb, colour(pool, *i).as_rgb()))
            .unwrap(),
    )
}
pub fn distance(a: [u8; 3], b: [u8; 3]) -> u32 {
    (0..3)
        .map(|i| (i32::from(a[i]) - i32::from(b[i])).pow(2) as u32)
        .sum()
}
pub fn resolve_name(pool: &ObjectPool, name: &str) -> Option<ResolvedColour> {
    named_rgb(name).map(|rgb| nearest(pool, rgb))
}
pub fn palette(pool: &ObjectPool) -> Vec<ResolvedColour> {
    (0..=255).map(|i| resolve_index(pool, i)).collect()
}
pub fn source(pool: &ObjectPool) -> String {
    let controls = pool
        .objects()
        .iter()
        .filter_map(|o| {
            if let Object::WorkingSetSpecialControls(c) = o {
                Some(c)
            } else {
                None
            }
        })
        .min_by_key(|c| c.id.value());
    controls.map(|c|format!("WorkingSetSpecialControls {}: ColourMap {:?}, ColourPalette {:?} (missing entries fall back to pool palette)",c.id.value(),c.id_of_colour_map.0.map(|i|i.value()),c.id_of_colour_palette.0.map(|i|i.value()))).unwrap_or_else(||"default VT pool palette".into())
}
/// WCAG relative-luminance contrast, used only as an editor readability heuristic.
pub fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
    fn luminance(rgb: [u8; 3]) -> f64 {
        let linear = rgb.map(|v| {
            let s = f64::from(v) / 255.;
            if s <= 0.04045 {
                s / 12.92
            } else {
                ((s + 0.055) / 1.055).powf(2.4)
            }
        });
        linear[0] * 0.2126 + linear[1] * 0.7152 + linear[2] * 0.0722
    }
    let (a, b) = (luminance(a), luminance(b));
    (a.max(b) + 0.05) / (a.min(b) + 0.05)
}
