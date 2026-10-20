//! Lossless indexed-PNG transport for PictureGraphic palette indices.
//!
//! PNG colour values alone are not enough here: two VT palette entries may have
//! identical RGB values while representing different authoring indices. These
//! helpers keep the index bytes in an indexed PNG and only accept the same
//! effective palette on import.
use crate::object_properties::{PropertyAccess, PropertyKey};
use ag_iso_stack::object_pool::{
    object::{Object, PictureGraphic},
    object_attributes::{DataCodeType, PictureGraphicFormat},
    ObjectPool, ObjectType,
};
use serde_json::Value;
use std::collections::BTreeMap;

fn decoded_indices(picture: &PictureGraphic) -> Result<Vec<u8>, String> {
    let width = usize::from(picture.actual_width);
    let height = usize::from(picture.actual_height);
    if width == 0 || height == 0 {
        return Err("PictureGraphic dimensions must be nonzero".into());
    }
    let pixels = width
        .checked_mul(height)
        .ok_or_else(|| "PictureGraphic dimensions overflow".to_owned())?;
    if pixels > png::Limits::default().bytes {
        return Err("PictureGraphic exceeds the PNG decoder's memory limit".into());
    }
    let per_row = match picture.format {
        PictureGraphicFormat::Monochrome => width.div_ceil(8),
        PictureGraphicFormat::FourBit => width.div_ceil(2),
        PictureGraphicFormat::EightBit => width,
    };
    let expected_bytes = per_row
        .checked_mul(height)
        .ok_or_else(|| "PictureGraphic dimensions overflow".to_owned())?;
    let raw = match picture.options.data_code_type {
        DataCodeType::Raw => picture.data.clone(),
        DataCodeType::RunLength => {
            if picture.data.len() % 2 != 0 {
                return Err("PictureGraphic has a truncated run-length pair".into());
            }
            let mut raw = Vec::new();
            for pair in picture.data.chunks_exact(2) {
                let count = usize::from(pair[0]);
                if count == 0 || raw.len().saturating_add(count) > expected_bytes {
                    return Err("PictureGraphic has invalid run-length data".into());
                }
                raw.resize(raw.len() + count, pair[1]);
            }
            raw
        }
    };
    if raw.len() < expected_bytes {
        return Err("PictureGraphic image data is shorter than its dimensions".into());
    }
    let mut indices = Vec::with_capacity(pixels);
    for row in 0..height {
        let bytes = &raw[row * per_row..(row + 1) * per_row];
        for column in 0..width {
            let index = match picture.format {
                PictureGraphicFormat::Monochrome => (bytes[column / 8] >> (7 - column % 8)) & 1,
                PictureGraphicFormat::FourBit => {
                    let byte = bytes[column / 2];
                    if column % 2 == 0 {
                        byte >> 4
                    } else {
                        byte & 0x0f
                    }
                }
                PictureGraphicFormat::EightBit => bytes[column],
            };
            indices.push(index);
        }
    }
    Ok(indices)
}

fn palette(pool: &ObjectPool) -> Vec<u8> {
    crate::effective_colour::colours(pool)
        .iter()
        .flat_map(|colour| colour.as_rgb())
        .collect()
}

fn transparency(picture: &PictureGraphic) -> Vec<u8> {
    (0..=255u8)
        .map(|index| {
            if picture.options.transparent && index == picture.transparency_colour {
                0
            } else {
                255
            }
        })
        .collect()
}

/// Encode the picture as an 8-bit indexed PNG, preserving every displayed index.
pub fn picture_png(picture: &PictureGraphic, pool: &ObjectPool) -> Result<Vec<u8>, String> {
    let indices = decoded_indices(picture)?;
    let mut output = Vec::new();
    {
        let mut encoder = png::Encoder::new(
            &mut output,
            u32::from(picture.actual_width),
            u32::from(picture.actual_height),
        );
        encoder.set_color(png::ColorType::Indexed);
        encoder.set_depth(png::BitDepth::Eight);
        encoder.set_palette(palette(pool));
        encoder.set_trns(transparency(picture));
        let mut writer = encoder.write_header().map_err(|error| error.to_string())?;
        writer
            .write_image_data(&indices)
            .map_err(|error| error.to_string())?;
    }
    Ok(output)
}

fn indices_from_png(
    bytes: &[u8],
    picture: &PictureGraphic,
    pool: &ObjectPool,
) -> Result<(u16, u16, Vec<u8>), String> {
    let decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    let mut reader = decoder.read_info().map_err(|error| error.to_string())?;
    if reader.output_color_type() != (png::ColorType::Indexed, png::BitDepth::Eight) {
        return Err("PictureGraphic input must be an 8-bit indexed PNG".into());
    }
    let info = reader.info();
    if info.palette.as_deref() != Some(palette(pool).as_slice()) {
        return Err("Indexed PNG palette does not match the project's effective VT palette".into());
    }
    let expected_transparency = transparency(picture);
    let encoded_transparency = info.trns.as_deref().unwrap_or(&[]);
    if !transparency_matches(encoded_transparency, &expected_transparency) {
        return Err("Indexed PNG transparency does not match the PictureGraphic".into());
    }
    if info.width == 0
        || info.height == 0
        || info.width > u16::MAX as u32
        || info.height > u16::MAX as u32
    {
        return Err("Indexed PNG dimensions must be between 1 and 65535".into());
    }
    let (png_width, png_height) = (info.width, info.height);
    let output_size = reader
        .output_buffer_size()
        .ok_or_else(|| "Indexed PNG output buffer size overflow".to_owned())?;
    if output_size > png::Limits::default().bytes {
        return Err("Indexed PNG exceeds the decoder's memory limit".into());
    }
    let mut output = vec![0; output_size];
    let frame = reader
        .next_frame(&mut output)
        .map_err(|error| error.to_string())?;
    output.truncate(frame.buffer_size());
    let pixels = usize::try_from(png_width)
        .ok()
        .and_then(|w| {
            usize::try_from(png_height)
                .ok()
                .and_then(|h| w.checked_mul(h))
        })
        .ok_or_else(|| "Indexed PNG dimensions overflow".to_owned())?;
    if output.len() != pixels {
        return Err("Indexed PNG did not decode to one index per pixel".into());
    }
    Ok((png_width as u16, png_height as u16, output))
}

fn transparency_matches(encoded: &[u8], expected: &[u8]) -> bool {
    (0..expected.len()).all(|index| encoded.get(index).copied().unwrap_or(255) == expected[index])
        && encoded.len() <= expected.len()
}

fn encode_indices(
    picture: &mut PictureGraphic,
    indices: &[u8],
    format: PictureGraphicFormat,
    encoding: DataCodeType,
) -> Result<(), String> {
    let width = usize::from(picture.actual_width);
    let height = usize::from(picture.actual_height);
    if indices.len() != width.saturating_mul(height) {
        return Err("Indexed PNG pixel count does not match the picture dimensions".into());
    }
    let mut raw = Vec::new();
    for row in indices.chunks_exact(width) {
        match format {
            PictureGraphicFormat::EightBit => raw.extend_from_slice(row),
            PictureGraphicFormat::FourBit => {
                for pair in row.chunks(2) {
                    if pair.iter().any(|index| *index > 15) {
                        return Err("FourBit PictureGraphic contains an index above 15".into());
                    }
                    raw.push((pair[0] << 4) | pair.get(1).copied().unwrap_or(0));
                }
            }
            PictureGraphicFormat::Monochrome => {
                for chunk in row.chunks(8) {
                    if chunk.iter().any(|index| *index > 1) {
                        return Err("Monochrome PictureGraphic contains an index above 1".into());
                    }
                    let mut byte = 0;
                    for (bit, index) in chunk.iter().enumerate() {
                        byte |= *index << (7 - bit);
                    }
                    raw.push(byte);
                }
            }
        }
    }
    picture.format = format;
    picture.options.data_code_type = encoding;
    picture.data = match encoding {
        DataCodeType::Raw => raw,
        DataCodeType::RunLength => {
            let mut encoded = Vec::new();
            let mut offset = 0;
            while offset < raw.len() {
                let value = raw[offset];
                let mut count = 1usize;
                while offset + count < raw.len() && raw[offset + count] == value && count < 255 {
                    count += 1;
                }
                encoded.extend([count as u8, value]);
                offset += count;
            }
            encoded
        }
    };
    Ok(())
}

/// Apply an indexed PNG while retaining the target's display and macro settings.
pub fn apply_png(
    picture: &mut PictureGraphic,
    pool: &ObjectPool,
    bytes: &[u8],
    format: PictureGraphicFormat,
    encoding: DataCodeType,
) -> Result<(), String> {
    let (width, height, indices) = indices_from_png(bytes, picture, pool)?;
    picture.actual_width = width;
    picture.actual_height = height;
    encode_indices(picture, &indices, format, encoding)
}

/// Return an error unless importing the PNG recreates the original image bytes.
pub fn verify_reproduces(
    picture: &PictureGraphic,
    pool: &ObjectPool,
    bytes: &[u8],
    format: PictureGraphicFormat,
    encoding: DataCodeType,
) -> Result<(), String> {
    if picture.format != format || picture.options.data_code_type != encoding {
        return Err(
            "Requested PictureGraphic format or compression differs from the source".into(),
        );
    }
    let mut reconstructed = picture.clone();
    apply_png(&mut reconstructed, pool, bytes, format, encoding)?;
    if reconstructed.actual_width != picture.actual_width
        || reconstructed.actual_height != picture.actual_height
        || reconstructed.data != picture.data
    {
        return Err("Indexed PNG does not reproduce the original PictureGraphic data bytes".into());
    }
    Ok(())
}

/// Update the image fields of a PictureGraphic component node from an indexed PNG.
/// The node's existing property-key set is retained exactly; a sparse PictureGraphic
/// that cannot represent the requested format is rejected instead of gaining defaults.
pub fn apply_png_to_component_properties(
    properties: &mut BTreeMap<PropertyKey, Value>,
    pool: &ObjectPool,
    bytes: &[u8],
    format: PictureGraphicFormat,
    encoding: DataCodeType,
) -> Result<(), String> {
    let updated_fields = [
        PropertyKey::Data,
        PropertyKey::ActualWidth,
        PropertyKey::ActualHeight,
        PropertyKey::Format,
        PropertyKey::Options,
    ];
    for field in [
        PropertyKey::ActualWidth,
        PropertyKey::ActualHeight,
        PropertyKey::Format,
        PropertyKey::Options,
    ] {
        if !properties.contains_key(&field) {
            return Err(format!(
                "PictureGraphic component node is missing required property '{field}'"
            ));
        }
    }

    let mut updated = properties.clone();
    let mut picture = picture_from_component_properties(&updated)?;
    apply_png(&mut picture, pool, bytes, format, encoding)?;
    for field in updated_fields {
        let value = picture
            .get_property(field)
            .map_err(|error| format!("PictureGraphic property '{field}': {error}"))?;
        updated.insert(field, value);
    }
    *properties = updated;
    Ok(())
}

/// Build a PictureGraphic from the typed properties retained by a component node.
pub fn picture_from_component_properties(
    properties: &BTreeMap<PropertyKey, Value>,
) -> Result<PictureGraphic, String> {
    let mut object = crate::default_object(ObjectType::PictureGraphic);
    let Object::PictureGraphic(picture) = &mut object else {
        unreachable!("PictureGraphic defaults must have PictureGraphic type")
    };
    for (property, value) in properties {
        picture
            .set_property(*property, value.clone())
            .map_err(|error| format!("PictureGraphic property '{property}': {error}"))?;
    }
    Ok((*picture).clone())
}
