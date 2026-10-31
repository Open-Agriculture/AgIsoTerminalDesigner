//! Authoritative editor rendering palette. Only explicitly referenced palette/map
//! objects are active; orphan palette objects never change the display. If malformed
//! pools have several special-controls objects, lowest object ID wins deterministically.
use ag_iso_stack::object_pool::{object::Object, Colour, ObjectPool};
use serde::{Deserialize, Serialize};

/// Resolves the active palette references once for a batch of colour lookups.
/// Missing references and entries retain the VT pool-palette fallback.
struct ColourResolver<'a> {
    pool: &'a ObjectPool,
    colour_map: Option<&'a [u8]>,
    palette: Option<&'a [Colour]>,
}

impl<'a> ColourResolver<'a> {
    fn new(pool: &'a ObjectPool) -> Self {
        let controls = pool
            .objects()
            .iter()
            .filter_map(|o| match o {
                Object::WorkingSetSpecialControls(c) => Some(c),
                _ => None,
            })
            .min_by_key(|c| c.id.value());
        let (colour_map, palette) = controls.map_or((None, None), |c| {
            let map = c
                .id_of_colour_map
                .0
                .and_then(|id| pool.object_by_id(id))
                .and_then(|o| match o {
                    Object::ColourMap(m) => Some(m.colour_map.as_slice()),
                    _ => None,
                });
            let palette = c
                .id_of_colour_palette
                .0
                .and_then(|id| pool.object_by_id(id))
                .and_then(|o| match o {
                    Object::ColourPalette(p) => Some(p.colours.as_slice()),
                    _ => None,
                });
            (map, palette)
        });
        Self {
            pool,
            colour_map,
            palette,
        }
    }

    fn colour(&self, index: u8) -> Colour {
        let mapped = self
            .colour_map
            .and_then(|map| map.get(index as usize).copied())
            .unwrap_or(index);
        self.palette
            .and_then(|palette| palette.get(mapped as usize).copied())
            .unwrap_or_else(|| self.pool.color_by_index(mapped))
    }
}

pub fn colour(pool: &ObjectPool, index: u8) -> Colour {
    ColourResolver::new(pool).colour(index)
}

/// The active palette's colours by palette index, before the Colour Map:
/// what a Colour Map entry points at.
pub fn palette_colours(pool: &ObjectPool) -> [Colour; 256] {
    let resolver = ColourResolver {
        colour_map: None,
        ..ColourResolver::new(pool)
    };
    std::array::from_fn(|index| resolver.colour(index as u8))
}

/// Effective RGBA colours indexed by the 8-bit VT colour index.
pub fn colours(pool: &ObjectPool) -> [Colour; 256] {
    let resolver = ColourResolver::new(pool);
    std::array::from_fn(|index| resolver.colour(index as u8))
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ResolvedColour {
    pub index: u8,
    pub name: String,
    pub rgb: String,
}
pub fn resolve_index(pool: &ObjectPool, index: u8) -> ResolvedColour {
    resolve_colour(index, colour(pool, index))
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
    let resolved = colours(pool);
    let index = (0..=255u8)
        .min_by_key(|i| distance(rgb, resolved[*i as usize].as_rgb()))
        .unwrap();
    resolve_colour(index, resolved[index as usize])
}
pub fn distance(a: [u8; 3], b: [u8; 3]) -> u32 {
    (0..3)
        .map(|i| (i32::from(a[i]) - i32::from(b[i])).pow(2) as u32)
        .sum()
}
pub fn resolve_name(pool: &ObjectPool, name: &str) -> Option<ResolvedColour> {
    named_rgb(name).map(|rgb| nearest(pool, rgb))
}
/// Resolve command text as an exact palette index, a colour name, or #RRGGBB.
/// Names and RGB values choose the nearest effective palette entry; explicit
/// indices retain their identity. Invalid text returns an actionable error.
pub fn resolve(pool: &ObjectPool, raw: &str) -> Result<ResolvedColour, String> {
    if let Ok(index) = raw.parse::<u8>() {
        return Ok(resolve_index(pool, index));
    }
    if raw.starts_with('#') {
        if raw.len() != 7 || !raw.is_ascii() {
            return Err("Colour must use #RRGGBB".into());
        }
        let channel = |start| {
            u8::from_str_radix(&raw[start..start + 2], 16)
                .map_err(|_| "Colour must use #RRGGBB".to_owned())
        };
        return Ok(nearest(pool, [channel(1)?, channel(3)?, channel(5)?]));
    }
    resolve_name(pool, raw).ok_or_else(|| {
        format!("Unknown colour '{raw}'; use a palette index, basic colour name, or #RRGGBB")
    })
}

pub fn palette(pool: &ObjectPool) -> Vec<ResolvedColour> {
    let resolved = colours(pool);
    (0..=255u8)
        .map(|index| resolve_colour(index, resolved[index as usize]))
        .collect()
}

fn resolve_colour(index: u8, colour: Colour) -> ResolvedColour {
    let rgb = colour.as_rgb();
    let name = (0..16u8)
        .find(|i| Colour::COLOUR_PALETTE[*i as usize].as_rgb() == rgb)
        .map(canonical_name)
        .unwrap_or_else(|| format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]));
    ResolvedColour {
        index,
        name,
        rgb: format!("#{:02X}{:02X}{:02X}", rgb[0], rgb[1], rgb[2]),
    }
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
