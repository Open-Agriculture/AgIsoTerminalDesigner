//! Project-local original images and single-owner source bindings.
use ag_iso_stack::object_pool::object::PictureGraphic;
pub use ag_iso_stack::object_pool::object_attributes::{DataCodeType, PictureGraphicFormat};
use ag_iso_stack::object_pool::{object::Object, ObjectId, ObjectPool};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub mod conversion;
pub mod conversion_ui;
pub mod indexed_png;
pub mod integration;
pub mod preview;
pub mod ui;

/// An image file as Graphic Data, whose only format is PNG (ISO 11783-6:2018,
/// Table B.74): a valid PNG is kept byte for byte, any other image the
/// `image` crate reads is converted to an 8-bit-per-channel PNG.
pub fn graphic_data_png(content: &[u8]) -> Result<Vec<u8>, String> {
    if crate::pool_validation::validate_graphic_data_png(content).is_ok() {
        return Ok(content.to_vec());
    }
    let image = image::load_from_memory(content)
        .map_err(|error| format!("The file is not an image this editor can read: {error}"))?;
    // Graphic Data allows at most 32 bits per pixel
    let image = if image.color().bits_per_pixel() > 32 {
        image::DynamicImage::ImageRgba8(image.to_rgba8())
    } else {
        image
    };
    let mut png = Vec::new();
    image
        .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
        .map_err(|error| format!("The image could not be converted to PNG: {error}"))?;
    crate::pool_validation::validate_graphic_data_png(&png)?;
    Ok(png)
}

#[cfg(test)]
mod graphic_data_tests {
    use super::graphic_data_png;

    fn encoded(image: image::DynamicImage, format: image::ImageFormat) -> Vec<u8> {
        let mut bytes = Vec::new();
        image
            .write_to(&mut std::io::Cursor::new(&mut bytes), format)
            .unwrap();
        bytes
    }

    #[test]
    fn png_is_kept_and_other_images_become_png() {
        let image = image::DynamicImage::ImageRgb8(image::RgbImage::new(3, 2));
        let png = encoded(image.clone(), image::ImageFormat::Png);
        assert_eq!(graphic_data_png(&png).unwrap(), png);

        let bmp = encoded(image, image::ImageFormat::Bmp);
        let converted = graphic_data_png(&bmp).unwrap();
        assert!(converted.starts_with(&[0x89, b'P', b'N', b'G']));
        let decoded = image::load_from_memory(&converted).unwrap();
        assert_eq!((decoded.width(), decoded.height()), (3, 2));
    }

    #[test]
    fn deep_images_are_reduced_to_eight_bits_per_channel() {
        let deep = image::DynamicImage::ImageRgba16(image::ImageBuffer::new(2, 2));
        let png = encoded(deep, image::ImageFormat::Png);
        let converted = graphic_data_png(&png).unwrap();
        assert_ne!(converted, png);
        assert!(crate::pool_validation::validate_graphic_data_png(&converted).is_ok());
    }

    #[test]
    fn other_files_are_rejected() {
        assert!(graphic_data_png(b"not an image").is_err());
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GraphicAssetId(pub uuid::Uuid);
impl Default for GraphicAssetId {
    fn default() -> Self {
        Self(uuid::Uuid::new_v4())
    }
}
impl std::fmt::Display for GraphicAssetId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GraphicAsset {
    pub id: GraphicAssetId,
    pub name: String,
    pub description: Option<String>,
    pub tags: Vec<String>,
    pub original_file_name: Option<String>,
    /// Decoder-recognized extension, independent of the uploaded filename.
    pub format: String,
    pub original_bytes: Vec<u8>,
}
impl GraphicAsset {
    pub fn from_upload(filename: &str, bytes: Vec<u8>) -> Result<Self, String> {
        decode(&bytes)?;
        let format = image::guess_format(&bytes).map_err(|e| e.to_string())?;
        let filename = std::path::Path::new(filename)
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("graphic");
        Ok(Self {
            id: GraphicAssetId::default(),
            name: std::path::Path::new(filename)
                .file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("Graphic")
                .into(),
            description: None,
            tags: vec![],
            original_file_name: Some(filename.into()),
            format: format
                .extensions_str()
                .first()
                .unwrap_or(&"img")
                .to_string(),
            original_bytes: bytes,
        })
    }
    pub fn download(&self) -> (String, &[u8]) {
        (
            self.original_file_name
                .clone()
                .unwrap_or_else(|| format!("{}.{}", self.name, self.format)),
            &self.original_bytes,
        )
    }

    /// Convert an available original to PNG without changing its authoring identity.
    pub fn as_png(&self) -> Result<Self, String> {
        let decoded = decode(&self.original_bytes)?;
        if matches!(
            decoded.color(),
            image::ColorType::Rgb32F | image::ColorType::Rgba32F
        ) {
            return Err(
                "Floating-point graphic pixels cannot be represented losslessly in PNG".into(),
            );
        }
        let decoded = decoded.to_rgba16();
        let mut output = std::io::Cursor::new(Vec::new());
        decoded
            .write_to(&mut output, image::ImageFormat::Png)
            .map_err(|error| error.to_string())?;
        let mut converted = self.clone();
        converted.format = "png".into();
        converted.original_bytes = output.into_inner();
        Ok(converted)
    }

    pub fn from_upload_with_id(
        filename: &str,
        bytes: Vec<u8>,
        id: GraphicAssetId,
    ) -> Result<Self, String> {
        let mut asset = Self::from_upload(filename, bytes)?;
        asset.id = id;
        Ok(asset)
    }
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GraphicLibrary {
    /// Uploaded source images waiting to be converted into PictureGraphic objects.
    pub assets: BTreeMap<GraphicAssetId, GraphicAsset>,
    /// Legacy development-project input. It is migrated during load and never
    /// emitted again, leaving persisted projects with only available assets.
    #[serde(default, rename = "picture_graphic_bindings", skip_serializing)]
    pub(crate) legacy_bindings: BTreeMap<u16, LegacyPictureGraphicBinding>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub(crate) struct LegacyPictureGraphicBinding {
    pub asset_id: GraphicAssetId,
    pub format: Option<PictureGraphicFormat>,
    #[serde(default)]
    pub transparent: Option<bool>,
    #[serde(default)]
    pub transparency_colour: Option<u8>,
    #[serde(default)]
    pub flashing: Option<bool>,
}
fn decode(content: &[u8]) -> Result<image::DynamicImage, String> {
    let img = image::load_from_memory(content).map_err(|e| e.to_string())?;
    if img.width() == 0
        || img.height() == 0
        || img.width() > u16::MAX as u32
        || img.height() > u16::MAX as u32
    {
        return Err("Image dimensions must be between 1 and 65535".into());
    }
    Ok(img)
}
/// The existing loader's colour-cube mapping; reserved indices remain untouched.
pub fn find_closest_color_index(r: u8, g: u8, b: u8) -> u8 {
    let q = |c: u8| ((c as u16 + 25) / 51).min(5) as u8;
    16 + 36 * q(r) + 6 * q(g) + q(b)
}
pub fn apply_image_bytes_to_picture_graphic(
    picture: &mut PictureGraphic,
    content: &[u8],
) -> Result<(), String> {
    let img = decode(content)?;
    picture.actual_width = img.width() as u16;
    picture.actual_height = img.height() as u16;
    if picture.width == 0 {
        picture.width = picture.actual_width;
    }
    picture.format = PictureGraphicFormat::EightBit;
    picture.transparency_colour = 1;
    picture.options.transparent = true;
    let rgba = if let Some(view) = img.as_rgba8() {
        std::borrow::Cow::Borrowed(view)
    } else {
        std::borrow::Cow::Owned(img.to_rgba8())
    };
    let mut raw = Vec::with_capacity(rgba.len() / 4);
    let mut rle = Vec::new();
    let mut count = 0u8;
    let mut value = 0;
    for p in rgba.pixels() {
        let idx = if p[3] == 0 {
            picture.transparency_colour
        } else {
            find_closest_color_index(p[0], p[1], p[2])
        };
        raw.push(idx);
        if count != 0 && (idx != value || count == u8::MAX) {
            rle.extend([count, value]);
            count = 0;
        }
        value = idx;
        count += 1;
    }
    if count != 0 {
        rle.extend([count, value]);
    }
    if rle.len() < raw.len() {
        picture.data = rle;
        picture.options.data_code_type = DataCodeType::RunLength;
    } else {
        picture.data = raw;
        picture.options.data_code_type = DataCodeType::Raw;
    }
    Ok(())
}

/// Convert indices between VT picture formats using the active palette.
/// Reduced formats reserve a target index for transparent pixels and map opaque
/// pixels to the nearest remaining target colour. `palette` is indexed by the
/// effective colour index after Colour Map and Colour Palette resolution.
pub fn convert_picture_graphic_format(
    picture: &mut PictureGraphic,
    target: PictureGraphicFormat,
    palette: &[[u8; 3]; 256],
) -> Result<(), String> {
    let options = conversion::ConversionOptions::suggested_with_palette(target, palette);
    let preview = conversion::preview_picture(picture, &options, palette)?;
    *picture = preview.picture;
    Ok(())
}

/// Current-encoding size statistics: index pixels, packed raw bytes, stored bytes.
pub fn picture_data_sizes(picture: &PictureGraphic) -> (usize, usize, usize) {
    indexed_png::picture_data_sizes(picture)
}

/// Validate the format restrictions for pictures used as shape fill patterns.
pub fn validate_picture_format(
    pool: &ObjectPool,
    picture_id: ObjectId,
    picture_width: u16,
    target: PictureGraphicFormat,
) -> Result<(), String> {
    let required_multiple = match target {
        PictureGraphicFormat::Monochrome => 8,
        PictureGraphicFormat::FourBit => 2,
        PictureGraphicFormat::EightBit => 1,
    };
    if picture_width % required_multiple == 0 {
        return Ok(());
    }
    let used_as_pattern = pool.objects().iter().any(|object| match object {
        Object::FillAttributes(fill) => {
            fill.fill_type == 3 && fill.fill_pattern.0 == Some(picture_id)
        }
        _ => false,
    });
    if used_as_pattern {
        Err(format!(
            "This PictureGraphic is used as a fill pattern; its width must be divisible by {required_multiple} in the selected format"
        ))
    } else {
        Ok(())
    }
}
