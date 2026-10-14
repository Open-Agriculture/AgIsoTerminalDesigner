//! Shared canvas, thumbnail, and PNG decoding and texture caching.
use crate::object_rendering::Colorable;
use ag_iso_stack::object_pool::{
    object::PictureGraphic, object_attributes::PictureGraphicFormat, ObjectPool,
};
use egui::{Color32, ColorImage, TextureHandle};
use std::hash::{Hash, Hasher};

pub fn decode_picture(picture: &PictureGraphic, pool: &ObjectPool) -> ColorImage {
    let palette = converted_palette(pool);
    decode_picture_with_palette(picture, &palette)
}

fn converted_palette(pool: &ObjectPool) -> [Color32; 256] {
    crate::effective_colour::colours(pool).map(|colour| colour.convert())
}

fn decode_picture_with_palette(picture: &PictureGraphic, palette: &[Color32; 256]) -> ColorImage {
    let transparent = palette[picture.transparency_colour as usize];
    let mut image = ColorImage::filled(
        [picture.actual_width.into(), picture.actual_height.into()],
        Color32::TRANSPARENT,
    );

    let width = picture.actual_width as usize;
    let mut pixel_index = 0;
    'bytes: for raw in picture.data_as_raw_encoded() {
        let count = match picture.format {
            PictureGraphicFormat::Monochrome => 8,
            PictureGraphicFormat::FourBit => 2,
            PictureGraphicFormat::EightBit => 1,
        };
        for slot in 0..count {
            if pixel_index >= image.pixels.len() {
                break 'bytes;
            }
            let colour_index = match picture.format {
                PictureGraphicFormat::Monochrome => (raw >> (7 - slot)) & 1,
                PictureGraphicFormat::FourBit => (raw >> (4 - slot * 4)) & 0x0f,
                PictureGraphicFormat::EightBit => raw,
            };
            let color = palette[colour_index as usize];
            if !picture.options.transparent || color != transparent {
                image.pixels[pixel_index] = color;
            }
            pixel_index += 1;
            // As before, unused bits after a row boundary are discarded.
            if width == 0 || pixel_index % width == 0 {
                continue 'bytes;
            }
        }
    }

    image
}

pub fn picture_texture(
    ctx: &egui::Context,
    picture: &PictureGraphic,
    pool: &ObjectPool,
) -> Option<TextureHandle> {
    let palette = converted_palette(pool);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    picture.id.value().hash(&mut hasher);
    picture.width.hash(&mut hasher);
    picture.actual_width.hash(&mut hasher);
    picture.actual_height.hash(&mut hasher);
    u8::from(picture.format).hash(&mut hasher);
    u8::from(picture.options).hash(&mut hasher);
    picture.transparency_colour.hash(&mut hasher);
    picture.data.hash(&mut hasher);
    picture.macro_refs.len().hash(&mut hasher);
    for macro_ref in &picture.macro_refs {
        macro_ref.macro_id.hash(&mut hasher);
        (macro_ref.event_id as u8).hash(&mut hasher);
    }
    for colour in palette {
        colour.to_array().hash(&mut hasher);
    }
    cached_texture(
        ctx,
        egui::Id::new(("picturegraphic_texture", picture.id.value())),
        hasher.finish(),
        || Ok(decode_picture_with_palette(picture, &palette)),
    )
}

pub fn asset_texture(ctx: &egui::Context, asset: &super::GraphicAsset) -> Option<TextureHandle> {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    asset.original_bytes.hash(&mut hasher);
    cached_texture(
        ctx,
        egui::Id::new(("available_texture", asset.id)),
        hasher.finish(),
        || {
            let image = super::decode(&asset.original_bytes)?.to_rgba8();
            Ok(ColorImage::from_rgba_unmultiplied(
                [image.width() as usize, image.height() as usize],
                image.as_raw(),
            ))
        },
    )
}

fn cached_texture(
    ctx: &egui::Context,
    key: egui::Id,
    hash: u64,
    decode: impl FnOnce() -> Result<ColorImage, String>,
) -> Option<TextureHandle> {
    // Keep the current and staged versions together so before/after previews
    // do not repeatedly decode the same two images or evict each other.
    let mut entries = ctx.data_mut(|data| {
        data.get_temp::<Vec<(u64, Option<TextureHandle>)>>(key)
            .unwrap_or_default()
    });
    if let Some((_, texture)) = entries.iter().find(|(old_hash, _)| *old_hash == hash) {
        return texture.clone();
    }
    let texture = decode()
        .ok()
        .filter(|image| !image.pixels.is_empty())
        .map(|image| ctx.load_texture(format!("graphics_{key:?}"), image, Default::default()));
    if entries.len() == 2 {
        entries.remove(0);
    }
    entries.push((hash, texture.clone()));
    ctx.data_mut(|data| data.insert_temp(key, entries));
    texture
}

pub fn picture_png(picture: &PictureGraphic, pool: &ObjectPool) -> Result<Vec<u8>, String> {
    let decoded = decode_picture(picture, pool);
    let bytes = decoded
        .pixels
        .iter()
        .flat_map(|color| color.to_srgba_unmultiplied())
        .collect();
    let image = image::RgbaImage::from_raw(decoded.size[0] as u32, decoded.size[1] as u32, bytes)
        .ok_or("Invalid graphic dimensions")?;
    let mut output = std::io::Cursor::new(Vec::new());
    image
        .write_to(&mut output, image::ImageFormat::Png)
        .map_err(|error| error.to_string())?;
    Ok(output.into_inner())
}
