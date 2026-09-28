//! Shared canvas, thumbnail, and PNG decoding and texture caching.
use crate::object_rendering::Colorable;
use ag_iso_stack::object_pool::{
    object::{Object, PictureGraphic},
    object_attributes::PictureGraphicFormat,
    ObjectPool,
};
use egui::{Color32, ColorImage, TextureHandle};
use std::hash::{Hash, Hasher};

pub fn decode_picture(picture: &PictureGraphic, pool: &ObjectPool) -> ColorImage {
    let mut x = 0;
    let mut y = 0;

    let mut image = ColorImage::filled(
        [picture.actual_width.into(), picture.actual_height.into()],
        Color32::TRANSPARENT,
    );

    for raw in picture.data_as_raw_encoded() {
        let mut colors: Vec<Color32> = vec![];
        match picture.format {
            PictureGraphicFormat::Monochrome => {
                for bit in 0..8 {
                    colors.push(
                        crate::effective_colour::colour(pool, (raw >> (7 - bit)) & 0x01).convert(),
                    );
                }
            }
            PictureGraphicFormat::FourBit => {
                for segment in 0..2 {
                    let shift = 4 - (segment * 4);
                    colors.push(
                        crate::effective_colour::colour(pool, (raw >> shift) & 0x0F).convert(),
                    );
                }
            }
            PictureGraphicFormat::EightBit => {
                colors.push(crate::effective_colour::colour(pool, raw).convert());
            }
        }

        for color in colors {
            let idx = y as usize * picture.actual_width as usize + x as usize;
            if idx >= image.pixels.len() {
                break;
            }
            if !(picture.options.transparent
                && color
                    == crate::effective_colour::colour(pool, picture.transparency_colour).convert())
            {
                image.pixels[idx] = color;
            }

            x += 1;
            if x >= picture.actual_width {
                x = 0;
                y += 1;
                // If we go onto the next row, then we discard the rest of the bits
                break;
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
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    Object::PictureGraphic(picture.clone())
        .write()
        .hash(&mut hasher);
    for index in 0..=255 {
        crate::effective_colour::colour(pool, index)
            .as_rgba()
            .hash(&mut hasher);
    }
    cached_texture(
        ctx,
        egui::Id::new(("picturegraphic_texture", picture.id.value())),
        hasher.finish(),
        || Ok(decode_picture(picture, pool)),
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
