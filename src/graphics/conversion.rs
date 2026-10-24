//! Shared, deterministic conversion of imported and indexed PictureGraphics.
use super::{indexed_png, PictureGraphicFormat};
use ag_iso_stack::object_pool::{object::PictureGraphic, object_attributes::DataCodeType};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MappingMethod {
    Nearest,
    LuminanceThreshold,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TransparencyMode {
    Preserve,
    Flatten,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum SourceColour {
    Index(u8),
    Rgba([u8; 4]),
}

impl Serialize for SourceColour {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let key = match self {
            Self::Index(index) => format!("index:{index}"),
            Self::Rgba(rgba) => format!(
                "rgba:{:02X}{:02X}{:02X}{:02X}",
                rgba[0], rgba[1], rgba[2], rgba[3]
            ),
        };
        serializer.serialize_str(&key)
    }
}

impl<'de> Deserialize<'de> for SourceColour {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let key = String::deserialize(deserializer)?;
        if let Some(index) = key.strip_prefix("index:") {
            return index
                .parse::<u8>()
                .map(Self::Index)
                .map_err(serde::de::Error::custom);
        }
        if let Some(hex) = key.strip_prefix("rgba:") {
            if hex.len() == 8 && hex.is_ascii() {
                let mut rgba = [0; 4];
                for (channel, offset) in [0, 2, 4, 6].into_iter().enumerate() {
                    rgba[channel] = u8::from_str_radix(&hex[offset..offset + 2], 16)
                        .map_err(serde::de::Error::custom)?;
                }
                return Ok(Self::Rgba(rgba));
            }
        }
        Err(serde::de::Error::custom(
            "source colour keys must be index:0..255 or rgba:RRGGBBAA",
        ))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ColourMapping {
    Index(u8),
    Transparent,
}

/// All user-adjustable inputs to picture conversion. Encoding is always selected
/// automatically from the packed raw and RLE payload sizes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ConversionOptions {
    pub target: PictureGraphicFormat,
    pub mapping: MappingMethod,
    pub automatic_threshold: bool,
    pub threshold: u8,
    pub invert: bool,
    pub transparency: TransparencyMode,
    /// An explicitly reserved output index. `None` allows an unused source key
    /// to be disabled; callers can also set an index when no transparent pixels
    /// are present to reserve it in the output.
    pub transparent_index: Option<u8>,
    pub flatten_background: u8,
    /// Imported alpha at or below this value is transparent in Preserve mode.
    pub alpha_cutoff: u8,
    pub mappings: BTreeMap<SourceColour, ColourMapping>,
}

impl ConversionOptions {
    pub fn suggested(target: PictureGraphicFormat) -> Self {
        Self {
            target,
            mapping: MappingMethod::Nearest,
            automatic_threshold: true,
            threshold: 128,
            invert: false,
            transparency: TransparencyMode::Preserve,
            transparent_index: None,
            flatten_background: 1,
            alpha_cutoff: 0,
            mappings: BTreeMap::new(),
        }
    }

    /// Build defaults using the active palette, including the preferred white
    /// flattening background when white is representable in the target range.
    pub fn suggested_with_palette(target: PictureGraphicFormat, palette: &[[u8; 3]; 256]) -> Self {
        let mut options = Self::suggested(target);
        let limit = format_limit(target);
        options.flatten_background = (0..limit)
            .find(|index| palette[*index] == [255; 3])
            .unwrap_or(1) as u8;
        options
    }
}

fn format_limit(target: PictureGraphicFormat) -> usize {
    match target {
        PictureGraphicFormat::Monochrome => 2,
        PictureGraphicFormat::FourBit => 16,
        PictureGraphicFormat::EightBit => 256,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SourceColourCount {
    pub source: SourceColour,
    pub pixels: usize,
    pub suggested: ColourMapping,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConversionPreview {
    pub picture: PictureGraphic,
    pub colours: Vec<SourceColourCount>,
    pub threshold: u8,
    pub raw_size: usize,
    pub rle_size: usize,
    pub stored_size: usize,
    pub rle_used: bool,
}

impl ConversionPreview {
    pub fn data_summary(&self) -> String {
        if self.rle_used {
            format!(
                "Data size: {} bytes (raw) optimized to {} bytes (RLE)",
                self.raw_size, self.stored_size
            )
        } else {
            format!(
                "Data size: {} bytes (raw; RLE provides no size reduction)",
                self.raw_size
            )
        }
    }

    pub fn data_tooltip(&self) -> String {
        if self.rle_size < self.raw_size {
            format!(
                "Packed raw: {} bytes; candidate RLE: {} bytes; saves {} bytes.",
                self.raw_size,
                self.rle_size,
                self.raw_size - self.rle_size
            )
        } else if self.rle_size == self.raw_size {
            format!(
                "Packed raw: {} bytes; candidate RLE: {} bytes; ties raw size, so raw is stored.",
                self.raw_size, self.rle_size
            )
        } else {
            format!("Packed raw: {} bytes; candidate RLE: {} bytes; increases size by {} bytes, so raw is stored.", self.raw_size, self.rle_size, self.rle_size - self.raw_size)
        }
    }
}

#[derive(Debug, Clone)]
enum Source {
    Indexed {
        width: u16,
        height: u16,
        indices: Vec<u8>,
        transparent: Option<u8>,
    },
    Rgba {
        width: u16,
        height: u16,
        pixels: Vec<[u8; 4]>,
    },
}

impl Source {
    fn dimensions(&self) -> (u16, u16) {
        match self {
            Self::Indexed { width, height, .. } | Self::Rgba { width, height, .. } => {
                (*width, *height)
            }
        }
    }
    fn samples(&self) -> Vec<(SourceColour, [u8; 3], u8)> {
        match self {
            Self::Indexed { indices, .. } => indices
                .iter()
                .copied()
                .map(|i| (SourceColour::Index(i), [0; 3], 255))
                .collect(),
            Self::Rgba { pixels, .. } => pixels
                .iter()
                .copied()
                .map(|p| (SourceColour::Rgba(p), [p[0], p[1], p[2]], p[3]))
                .collect(),
        }
    }
}

/// Produce the exact preview/application result for an existing indexed picture.
pub fn preview_picture(
    picture: &PictureGraphic,
    options: &ConversionOptions,
    palette: &[[u8; 3]; 256],
) -> Result<ConversionPreview, String> {
    let source = Source::Indexed {
        width: picture.actual_width,
        height: picture.actual_height,
        indices: indexed_png::pixel_indices(picture)?,
        transparent: picture
            .options
            .transparent
            .then_some(picture.transparency_colour),
    };
    convert(source, Some(picture), options, palette)
}

/// Produce the exact preview/application result for a decoded image file.
pub fn preview_image(
    picture_template: &PictureGraphic,
    bytes: &[u8],
    options: &ConversionOptions,
    palette: &[[u8; 3]; 256],
) -> Result<ConversionPreview, String> {
    let image = image::load_from_memory(bytes).map_err(|e| e.to_string())?;
    if image.width() == 0
        || image.height() == 0
        || image.width() > u16::MAX as u32
        || image.height() > u16::MAX as u32
    {
        return Err("Image dimensions must be between 1 and 65535".into());
    }
    let rgba = image.to_rgba8();
    let pixels = rgba.pixels().map(|p| [p[0], p[1], p[2], p[3]]).collect();
    convert(
        Source::Rgba {
            width: image.width() as u16,
            height: image.height() as u16,
            pixels,
        },
        Some(picture_template),
        options,
        palette,
    )
}

/// Apply conversion by replacing a picture with the preview result.
pub fn apply_picture_conversion(
    picture: &mut PictureGraphic,
    options: &ConversionOptions,
    palette: &[[u8; 3]; 256],
) -> Result<(), String> {
    let preview = preview_picture(picture, options, palette)?;
    *picture = preview.picture;
    Ok(())
}

/// Return the candidate RLE size for the current picture's packed raw data.
pub fn candidate_rle_size(picture: &PictureGraphic) -> Result<usize, String> {
    let indices = indexed_png::pixel_indices(picture)?;
    let mut candidate = picture.clone();
    indexed_png::encode_indices(
        &mut candidate,
        &indices,
        picture.format,
        DataCodeType::RunLength,
    )?;
    Ok(candidate.data.len())
}

fn convert(
    source: Source,
    template: Option<&PictureGraphic>,
    options: &ConversionOptions,
    palette: &[[u8; 3]; 256],
) -> Result<ConversionPreview, String> {
    if options.mapping == MappingMethod::LuminanceThreshold
        && options.target != PictureGraphicFormat::Monochrome
    {
        return Err("Luminance threshold mapping is available only for Monochrome output".into());
    }
    let (width, height) = source.dimensions();
    let samples = source.samples();
    let limit = format_limit(options.target);
    let mut counts: BTreeMap<SourceColour, (usize, [u8; 3], u8)> = BTreeMap::new();
    for (key, rgb, alpha) in &samples {
        let entry = counts.entry(*key).or_insert((0, *rgb, *alpha));
        entry.0 += 1;
    }
    let mut threshold = options.threshold;
    if options.mapping == MappingMethod::LuminanceThreshold && options.automatic_threshold {
        threshold = suggested_threshold(&samples, palette);
    }
    let source_transparency = match &source {
        Source::Indexed { transparent, .. } => *transparent,
        _ => None,
    };
    let mut transparent_pixels = vec![false; samples.len()];
    for (i, (key, _, alpha)) in samples.iter().enumerate() {
        let inherited = match (&source, options.transparency) {
            (Source::Indexed { .. }, TransparencyMode::Preserve) => {
                source_transparency
                    == Some(match key {
                        SourceColour::Index(v) => *v,
                        _ => 0,
                    })
            }
            (Source::Rgba { .. }, TransparencyMode::Preserve) => *alpha <= options.alpha_cutoff,
            _ => false,
        };
        transparent_pixels[i] = match options.mappings.get(key) {
            Some(ColourMapping::Index(_)) => false,
            Some(ColourMapping::Transparent) => options.transparency == TransparencyMode::Preserve,
            None => inherited,
        };
    }
    let has_transparency = transparent_pixels.iter().any(|v| *v);
    let has_transparent_override = counts
        .keys()
        .any(|source| options.mappings.get(source) == Some(&ColourMapping::Transparent));
    let reserved = if options.transparency == TransparencyMode::Preserve {
        options.transparent_index
    } else {
        None
    };
    let needs_transparency = has_transparency || has_transparent_override;
    let transparency_index = reserved.or_else(|| {
        if needs_transparency {
            suggest_transparency_index(
                limit,
                &samples,
                &transparent_pixels,
                palette,
                options.target,
                options.mapping,
                threshold,
                options.invert,
                &options.mappings,
            )
        } else {
            None
        }
    });
    let active_transparency =
        transparency_index.filter(|_| needs_transparency || reserved.is_some());
    let candidates: Vec<u8> = (0..limit)
        .map(|index| index as u8)
        .filter(|index| active_transparency != Some(*index))
        .collect();
    if candidates.is_empty() {
        return Err("No target colours remain after reserving transparency".into());
    }
    if let Some(index) = transparency_index {
        if usize::from(index) >= limit {
            return Err(format!(
                "Transparent output index {index} is outside the selected format"
            ));
        }
    }
    if usize::from(options.flatten_background) >= limit {
        return Err(format!(
            "Flattening background index {} is outside the selected format",
            options.flatten_background
        ));
    }

    let auto_mapping = |source_key: SourceColour, rgb: [u8; 3]| -> u8 {
        if options.target == PictureGraphicFormat::Monochrome
            && options.mapping == MappingMethod::LuminanceThreshold
        {
            let luminance =
                ((u32::from(rgb[0]) * 299 + u32::from(rgb[1]) * 587 + u32::from(rgb[2]) * 114)
                    / 1000) as u8;
            let white = luminance >= threshold;
            let dark = *candidates
                .iter()
                .min_by_key(|index| rgb_luminance(palette[usize::from(**index)]))
                .unwrap();
            let light = *candidates
                .iter()
                .max_by_key(|index| rgb_luminance(palette[usize::from(**index)]))
                .unwrap();
            if white ^ options.invert {
                light
            } else {
                dark
            }
        } else {
            if let SourceColour::Index(index) = source_key {
                let flattening_transparency = options.transparency == TransparencyMode::Flatten
                    && source_transparency == Some(index);
                if candidates.contains(&index) && !flattening_transparency {
                    return index;
                }
            }
            *candidates
                .iter()
                .min_by_key(|index| distance(rgb, palette[usize::from(**index)]))
                .unwrap()
        }
    };

    let mut output = Vec::with_capacity(samples.len());
    let mut suggestions = BTreeMap::new();
    for (i, (key, mut rgb, alpha)) in samples.iter().copied().enumerate() {
        if options.transparency == TransparencyMode::Flatten
            && matches!(source, Source::Rgba { .. })
            && alpha < 255
        {
            let bg = palette[usize::from(options.flatten_background)];
            rgb = std::array::from_fn(|c| {
                ((u16::from(rgb[c]) * u16::from(alpha)
                    + u16::from(bg[c]) * u16::from(255 - alpha)
                    + 127)
                    / 255) as u8
            });
        } else if let SourceColour::Index(index) = key {
            rgb = if options.transparency == TransparencyMode::Flatten
                && source_transparency == Some(index)
            {
                palette[usize::from(options.flatten_background)]
            } else {
                palette[usize::from(index)]
            };
        }
        let manual = options.mappings.get(&key).copied();
        let is_transparent =
            transparent_pixels[i] && options.transparency == TransparencyMode::Preserve;
        let suggestion = if is_transparent {
            ColourMapping::Transparent
        } else {
            ColourMapping::Index(auto_mapping(key, rgb))
        };
        suggestions.insert(key, suggestion);
        let mapped = if let Some(manual) = manual {
            manual
        } else if is_transparent {
            ColourMapping::Transparent
        } else {
            suggestion
        };
        let index = match mapped {
            ColourMapping::Transparent if options.transparency == TransparencyMode::Preserve => {
                transparency_index.ok_or_else(|| {
                    "Transparent mapping requires an output transparency index".to_owned()
                })?
            }
            ColourMapping::Transparent => {
                return Err("Transparent colour mappings require Preserve transparency".into())
            }
            ColourMapping::Index(index)
                if usize::from(index) < limit && active_transparency != Some(index) =>
            {
                index
            }
            ColourMapping::Index(index) if active_transparency == Some(index) => {
                return Err(format!(
                    "Opaque mapping uses reserved transparent index {index}"
                ))
            }
            ColourMapping::Index(index) => {
                return Err(format!(
                    "Mapped target index {index} is outside the selected format"
                ))
            }
        };
        output.push(index);
    }
    let mut result = template
        .cloned()
        .ok_or_else(|| "A PictureGraphic template is required".to_owned())?;
    result.actual_width = width;
    result.actual_height = height;
    if result.width == 0 {
        result.width = width;
    }
    result.format = options.target;
    result.options.transparent = active_transparency.is_some();
    if let Some(key) = active_transparency {
        result.transparency_colour = key;
    } else if usize::from(result.transparency_colour) >= limit {
        result.transparency_colour = 0;
    }
    indexed_png::encode_indices_optimized(&mut result, &output, options.target)?;
    let (_, raw_size, stored_size) = indexed_png::picture_data_sizes(&result);
    let mut rle_candidate = result.clone();
    indexed_png::encode_indices(
        &mut rle_candidate,
        &output,
        options.target,
        DataCodeType::RunLength,
    )?;
    let rle_size = rle_candidate.data.len();
    let colour_rows = counts
        .into_iter()
        .map(|(source, (pixels, _, _))| {
            let suggested = suggestions
                .get(&source)
                .copied()
                .unwrap_or_else(|| match source {
                    SourceColour::Index(index) if source_transparency == Some(index) => {
                        ColourMapping::Transparent
                    }
                    _ => ColourMapping::Index(0),
                });
            SourceColourCount {
                source,
                pixels,
                suggested,
            }
        })
        .collect();
    Ok(ConversionPreview {
        picture: result,
        colours: colour_rows,
        threshold,
        raw_size,
        rle_size,
        stored_size,
        rle_used: rle_size < raw_size,
    })
}

fn suggest_transparency_index(
    limit: usize,
    samples: &[(SourceColour, [u8; 3], u8)],
    transparent: &[bool],
    palette: &[[u8; 3]; 256],
    target: PictureGraphicFormat,
    method: MappingMethod,
    threshold: u8,
    invert: bool,
    overrides: &BTreeMap<SourceColour, ColourMapping>,
) -> Option<u8> {
    let all: Vec<u8> = (0..limit).map(|index| index as u8).collect();
    let mut frequencies = vec![0usize; limit];
    let mut histogram: BTreeMap<[u8; 3], usize> = BTreeMap::new();
    let mut fixed = BTreeSet::new();
    for (i, (key, rgb, _)) in samples.iter().enumerate() {
        if transparent[i] {
            continue;
        }
        let rgb = match key {
            SourceColour::Index(index) => palette[usize::from(*index)],
            _ => *rgb,
        };
        let mapped = match overrides.get(key) {
            Some(ColourMapping::Index(index)) if usize::from(*index) < limit => {
                fixed.insert(usize::from(*index));
                usize::from(*index)
            }
            Some(ColourMapping::Index(_)) | Some(ColourMapping::Transparent) => continue,
            None => match key {
                _ if target == PictureGraphicFormat::Monochrome
                    && method == MappingMethod::LuminanceThreshold =>
                {
                    let luma = rgb_luminance(rgb) as u8;
                    let dark = *all
                        .iter()
                        .min_by_key(|index| rgb_luminance(palette[usize::from(**index)]))
                        .unwrap();
                    let light = *all
                        .iter()
                        .max_by_key(|index| rgb_luminance(palette[usize::from(**index)]))
                        .unwrap();
                    usize::from(if (luma >= threshold) ^ invert {
                        light
                    } else {
                        dark
                    })
                }
                SourceColour::Index(index) if usize::from(*index) < limit => usize::from(*index),
                _ => usize::from(
                    *all.iter()
                        .min_by_key(|index| distance(rgb, palette[usize::from(**index)]))
                        .unwrap(),
                ),
            },
        };
        frequencies[mapped] += 1;
        if overrides.get(key).is_none() {
            *histogram.entry(rgb).or_default() += 1;
        }
    }
    if let Some(unused) = frequencies.iter().position(|count| *count == 0) {
        return Some(unused as u8);
    }
    let white_targets: BTreeSet<usize> =
        if target != PictureGraphicFormat::Monochrome && histogram.contains_key(&[255; 3]) {
            all.iter()
                .enumerate()
                .filter_map(|(index, palette_index)| {
                    (palette[usize::from(*palette_index)] == [255; 3]).then_some(index)
                })
                .collect()
        } else {
            BTreeSet::new()
        };
    let mut baseline = 0u64;
    let mut reservation_penalty = vec![0u64; limit];
    for (rgb, count) in &histogram {
        let mut best = (u32::MAX, usize::MAX);
        let mut second = (u32::MAX, usize::MAX);
        for (index, palette_index) in all.iter().enumerate() {
            let candidate = (distance(*rgb, palette[usize::from(*palette_index)]), index);
            if candidate < best {
                second = best;
                best = candidate;
            } else if candidate < second {
                second = candidate;
            }
        }
        let frequency = *count as u64;
        baseline += u64::from(best.0) * frequency;
        reservation_penalty[best.1] += u64::from(second.0 - best.0) * frequency;
    }
    let costs: Vec<(u64, usize)> = (0..limit)
        .map(|candidate| {
            let error = if fixed.contains(&candidate) {
                u64::MAX
            } else {
                baseline + reservation_penalty[candidate]
            };
            (error, candidate)
        })
        .collect();
    if costs.iter().all(|(error, _)| *error == u64::MAX) {
        return None;
    }
    let tie_break = |candidate: &&(u64, usize)| {
        let (error, index) = **candidate;
        let palette_tie = if target == PictureGraphicFormat::Monochrome {
            usize::from(rgb_luminance(palette[index]))
        } else {
            index
        };
        (error, palette_tie, index)
    };
    costs
        .iter()
        .filter(|(error, candidate)| *error != u64::MAX && !white_targets.contains(candidate))
        .min_by_key(tie_break)
        .or_else(|| {
            costs
                .iter()
                .filter(|(error, _)| *error != u64::MAX)
                .min_by_key(tie_break)
        })
        .map(|(_, index)| *index as u8)
}

fn rgb_luminance(rgb: [u8; 3]) -> u16 {
    ((u32::from(rgb[0]) * 299 + u32::from(rgb[1]) * 587 + u32::from(rgb[2]) * 114) / 1000) as u16
}

fn suggested_threshold(samples: &[(SourceColour, [u8; 3], u8)], palette: &[[u8; 3]; 256]) -> u8 {
    let values: Vec<u8> = samples
        .iter()
        .filter_map(|(key, rgb, alpha)| {
            if *alpha == 0 {
                return None;
            }
            let rgb = match key {
                SourceColour::Index(i) => palette[usize::from(*i)],
                _ => *rgb,
            };
            Some(
                ((u32::from(rgb[0]) * 299 + u32::from(rgb[1]) * 587 + u32::from(rgb[2]) * 114)
                    / 1000) as u8,
            )
        })
        .collect();
    if values.is_empty() {
        return 128;
    }
    let mut ordered = values;
    ordered.sort_unstable();
    ordered[ordered.len() / 2]
}

fn distance(a: [u8; 3], b: [u8; 3]) -> u32 {
    (0..3)
        .map(|i| (i32::from(a[i]) - i32::from(b[i])).unsigned_abs().pow(2))
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;
    use ag_iso_stack::object_pool::{object::Object, ObjectType};

    fn template() -> PictureGraphic {
        let object = crate::default_object(ObjectType::PictureGraphic);
        let Object::PictureGraphic(picture) = object else {
            unreachable!()
        };
        picture
    }

    fn rgba_png(pixel: [u8; 4]) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(1, 1, image::Rgba(pixel));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    fn rgba_pixels_png(pixels: &[[u8; 4]]) -> Vec<u8> {
        let mut image = image::RgbaImage::new(pixels.len() as u32, 1);
        for (x, pixel) in pixels.iter().enumerate() {
            image.put_pixel(x as u32, 0, image::Rgba(*pixel));
        }
        let mut bytes = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(image)
            .write_to(&mut bytes, image::ImageFormat::Png)
            .unwrap();
        bytes.into_inner()
    }

    fn palette() -> [[u8; 3]; 256] {
        let mut colors = [[0; 3]; 256];
        colors[1] = [255; 3];
        colors[255] = [255, 0, 0];
        colors
    }

    #[test]
    fn conversion_options_serialize_mapping_keys_as_strings() {
        let mut options = ConversionOptions::suggested(PictureGraphicFormat::EightBit);
        options
            .mappings
            .insert(SourceColour::Index(231), ColourMapping::Index(1));
        options.mappings.insert(
            SourceColour::Rgba([255, 255, 255, 255]),
            ColourMapping::Transparent,
        );
        let json = serde_json::to_string(&options).unwrap();
        assert!(json.contains("index:231"));
        assert!(json.contains("rgba:FFFFFFFF"));
        assert_eq!(
            serde_json::from_str::<ConversionOptions>(&json).unwrap(),
            options
        );
    }

    #[test]
    fn eight_bit_import_supports_index_255_and_transparent_override() {
        let colors = palette();
        let mut options = ConversionOptions::suggested(PictureGraphicFormat::EightBit);
        options.mappings.insert(
            SourceColour::Rgba([255, 0, 0, 255]),
            ColourMapping::Index(255),
        );
        let preview =
            preview_image(&template(), &rgba_png([255, 0, 0, 255]), &options, &colors).unwrap();
        assert_eq!(preview.colours[0].suggested, ColourMapping::Index(255));
        assert_eq!(
            indexed_png::pixel_indices(&preview.picture).unwrap(),
            vec![255]
        );

        options.mappings.insert(
            SourceColour::Rgba([255, 0, 0, 255]),
            ColourMapping::Transparent,
        );
        let preview =
            preview_image(&template(), &rgba_png([255, 0, 0, 255]), &options, &colors).unwrap();
        assert!(preview.picture.options.transparent);
        assert_eq!(
            indexed_png::pixel_indices(&preview.picture).unwrap(),
            vec![preview.picture.transparency_colour]
        );
    }

    #[test]
    fn transparent_key_selection_keeps_exact_opaque_white() {
        let mut colors = [[0; 3]; 256];
        let mut pixels = Vec::new();
        for index in 0..16u8 {
            let rgb = match index {
                0 => [0, 0, 0],
                1 => [255; 3],
                n => [n * 11, n * 7, n * 3],
            };
            colors[usize::from(index)] = rgb;
            pixels.push([rgb[0], rgb[1], rgb[2], 255]);
        }
        pixels.push([0, 0, 0, 0]);
        let mut options = ConversionOptions::suggested(PictureGraphicFormat::FourBit);
        options.transparent_index = None;
        let preview =
            preview_image(&template(), &rgba_pixels_png(&pixels), &options, &colors).unwrap();
        let mapped = indexed_png::pixel_indices(&preview.picture).unwrap();
        assert_ne!(preview.picture.transparency_colour, 1);
        assert_eq!(mapped[1], 1);
    }

    #[test]
    fn transparent_monochrome_retains_best_fitting_opaque_colour_and_white_on_ties() {
        let colors = palette();
        let options = ConversionOptions::suggested(PictureGraphicFormat::Monochrome);
        for (black_pixels, expected_key) in [(1, 0), (10, 1)] {
            let mut pixels = vec![[0, 0, 0, 255]; black_pixels];
            pixels.extend([[255, 255, 255, 255], [0, 0, 0, 0]]);
            let preview =
                preview_image(&template(), &rgba_pixels_png(&pixels), &options, &colors).unwrap();
            assert_eq!(preview.picture.transparency_colour, expected_key);
        }
    }

    #[test]
    fn flattening_blends_partial_alpha_and_indexed_transparency() {
        let mut colors = palette();
        colors[2] = [127, 127, 255];
        let mut options = ConversionOptions::suggested(PictureGraphicFormat::EightBit);
        options.transparency = TransparencyMode::Flatten;
        options.flatten_background = 1;
        let preview =
            preview_image(&template(), &rgba_png([0, 0, 255, 128]), &options, &colors).unwrap();
        assert_eq!(
            indexed_png::pixel_indices(&preview.picture).unwrap(),
            vec![2]
        );
        assert!(!preview.picture.options.transparent);

        let mut indexed = template();
        indexed.actual_width = 2;
        indexed.actual_height = 1;
        indexed.format = PictureGraphicFormat::EightBit;
        indexed.options.transparent = true;
        indexed.transparency_colour = 2;
        indexed.data = vec![2, 3];
        let preview = preview_picture(&indexed, &options, &colors).unwrap();
        assert_eq!(
            indexed_png::pixel_indices(&preview.picture).unwrap(),
            vec![1, 3]
        );
        assert!(!preview.picture.options.transparent);
    }

    #[test]
    fn threshold_and_inversion_follow_active_monochrome_palette() {
        let mut colors = palette();
        colors[0] = [255; 3];
        colors[1] = [0; 3];
        let bytes = rgba_pixels_png(&[[255, 255, 255, 255], [0, 0, 0, 255]]);
        let mut options = ConversionOptions::suggested(PictureGraphicFormat::Monochrome);
        options.mapping = MappingMethod::LuminanceThreshold;
        options.automatic_threshold = false;
        options.threshold = 128;
        let preview = preview_image(&template(), &bytes, &options, &colors).unwrap();
        assert_eq!(
            indexed_png::pixel_indices(&preview.picture).unwrap(),
            vec![0, 1]
        );
        options.invert = true;
        let preview = preview_image(&template(), &bytes, &options, &colors).unwrap();
        assert_eq!(
            indexed_png::pixel_indices(&preview.picture).unwrap(),
            vec![1, 0]
        );
        options.target = PictureGraphicFormat::FourBit;
        assert!(preview_image(&template(), &bytes, &options, &colors).is_err());
    }

    #[test]
    fn white_stays_white_across_sequential_reductions_and_duplicate_indices() {
        let mut colors = palette();
        colors[0] = [0; 3];
        colors[1] = [255; 3];
        colors[2] = [255, 0, 0];
        colors[3] = colors[2];
        let mut picture = template();
        picture.actual_width = 2;
        picture.actual_height = 1;
        picture.format = PictureGraphicFormat::EightBit;
        picture.data = vec![1, 2];

        let four_options = ConversionOptions::suggested(PictureGraphicFormat::FourBit);
        let four = preview_picture(&picture, &four_options, &colors)
            .unwrap()
            .picture;
        assert_eq!(indexed_png::pixel_indices(&four).unwrap(), vec![1, 2]);
        let mono_options = ConversionOptions::suggested(PictureGraphicFormat::Monochrome);
        let mono = preview_picture(&four, &mono_options, &colors)
            .unwrap()
            .picture;
        assert_eq!(indexed_png::pixel_indices(&mono).unwrap(), vec![1, 0]);

        picture.data = vec![2, 3];
        let options = ConversionOptions::suggested(PictureGraphicFormat::EightBit);
        let duplicate_indices = preview_picture(&picture, &options, &colors)
            .unwrap()
            .picture;
        assert_eq!(
            indexed_png::pixel_indices(&duplicate_indices).unwrap(),
            vec![2, 3]
        );
    }

    #[test]
    fn packed_raw_summary_counts_odd_width_row_padding() {
        let mut picture = template();
        picture.actual_width = 3;
        picture.actual_height = 1;
        picture.format = PictureGraphicFormat::Monochrome;
        picture.data = vec![0b1010_0000];
        let options = ConversionOptions::suggested(PictureGraphicFormat::Monochrome);
        let preview = preview_picture(&picture, &options, &palette()).unwrap();
        assert_eq!(preview.raw_size, 1);
        assert_eq!(preview.rle_size, 2);
        assert_eq!(
            preview.data_summary(),
            "Data size: 1 bytes (raw; RLE provides no size reduction)"
        );
        assert!(preview.data_tooltip().contains("increases size by 1 bytes"));
    }

    #[test]
    fn rle_is_used_only_when_strictly_smaller_than_packed_raw() {
        let options = ConversionOptions::suggested(PictureGraphicFormat::Monochrome);
        let colors = palette();
        let mut repeated = template();
        repeated.actual_width = 80;
        repeated.actual_height = 1;
        repeated.format = PictureGraphicFormat::Monochrome;
        repeated.data = vec![0; 10];
        let preview = preview_picture(&repeated, &options, &colors).unwrap();
        assert_eq!(
            (preview.raw_size, preview.rle_size, preview.stored_size),
            (10, 2, 2)
        );
        assert!(preview.rle_used);

        let mut tied = repeated;
        tied.actual_width = 16;
        tied.data = vec![0; 2];
        let preview = preview_picture(&tied, &options, &colors).unwrap();
        assert_eq!(
            (preview.raw_size, preview.rle_size, preview.stored_size),
            (2, 2, 2)
        );
        assert!(!preview.rle_used);
        assert_eq!(
            preview.data_summary(),
            "Data size: 2 bytes (raw; RLE provides no size reduction)"
        );
        assert!(preview.data_tooltip().contains("ties raw size"));
    }
}
