//! Project-local original images and single-owner source bindings.
use ag_iso_stack::object_pool::{
    object::PictureGraphic,
    object_attributes::{DataCodeType, PictureGraphicFormat},
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
pub mod integration;
pub mod ui;

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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PictureGraphicSource {
    Primitive {
        object_id: u16,
    },
    ComponentNode {
        instance_id: uuid::Uuid,
        node_id: uuid::Uuid,
    },
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
}
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PictureGraphicBinding {
    pub asset_id: GraphicAssetId,
    /// None uses the loader's EightBit default. Never alters canonical raw data.
    pub format: Option<PictureGraphicFormat>,
    #[serde(default)]
    pub transparent: Option<bool>,
    #[serde(default)]
    pub transparency_colour: Option<u8>,
    #[serde(default)]
    pub flashing: Option<bool>,
}
impl From<GraphicAssetId> for PictureGraphicBinding {
    fn from(asset_id: GraphicAssetId) -> Self {
        Self {
            asset_id,
            format: None,
            transparent: None,
            transparency_colour: None,
            flashing: None,
        }
    }
}
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct GraphicLibrary {
    pub assets: BTreeMap<GraphicAssetId, GraphicAsset>,
    /// Ordinary source bindings. Component bindings are declared parameter values,
    /// resolved into the same ownership index by the project materializer.
    pub picture_graphic_bindings: BTreeMap<u16, PictureGraphicBinding>,
}
impl GraphicLibrary {
    pub(crate) fn set_binding(&mut self, object_id: u16, asset: Option<GraphicAssetId>) {
        if let Some(asset) = asset {
            if self
                .picture_graphic_bindings
                .get(&object_id)
                .is_none_or(|b| b.asset_id != asset)
            {
                self.picture_graphic_bindings
                    .insert(object_id, asset.into());
            }
        } else {
            self.picture_graphic_bindings.remove(&object_id);
        }
    }
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

/// Preserve the legacy radio-button packing semantics, including incomplete chunks.
pub fn convert_picture_graphic_format(picture: &mut PictureGraphic, target: PictureGraphicFormat) {
    match target {
        PictureGraphicFormat::Monochrome => match picture.format {
            PictureGraphicFormat::FourBit => {
                picture.data = picture
                    .data_as_raw_encoded()
                    .windows(4)
                    .step_by(4)
                    .flat_map(|chunk| {
                        let mut byte = 0;
                        for (i, bit) in chunk.iter().enumerate() {
                            for j in 0..2 {
                                if *bit & (1 << j) != 0 {
                                    byte |= 1 << (i * 2 + j);
                                }
                            }
                        }
                        vec![byte]
                    })
                    .collect();
            }
            PictureGraphicFormat::EightBit => {
                picture.data = picture
                    .data_as_raw_encoded()
                    .windows(8)
                    .step_by(8)
                    .flat_map(|chunk| {
                        let mut byte = 0;
                        for (i, bit) in chunk.iter().enumerate() {
                            if *bit != 0 {
                                byte |= 1 << i;
                            }
                        }
                        vec![byte]
                    })
                    .collect();
            }
            _ => {}
        },
        PictureGraphicFormat::FourBit => match picture.format {
            PictureGraphicFormat::Monochrome => {
                picture.data = picture
                    .data_as_raw_encoded()
                    .iter()
                    .flat_map(|value| {
                        let mut result = vec![];
                        for idx in 0..8 {
                            let bit_color = value << idx & 0x01;
                            if idx % 2 == 0 {
                                result.push(bit_color);
                            } else if let Some(last) = result.last_mut() {
                                *last |= bit_color >> 4;
                            }
                        }
                        result
                    })
                    .collect();
            }
            PictureGraphicFormat::EightBit => {
                picture.data = picture
                    .data_as_raw_encoded()
                    .windows(2)
                    .step_by(2)
                    .flat_map(|values| {
                        let high = (values[0] & 0x0F) << 4;
                        let low = values[1] & 0x0F;
                        vec![high | low]
                    })
                    .collect();
            }
            _ => {}
        },
        PictureGraphicFormat::EightBit => match picture.format {
            PictureGraphicFormat::Monochrome => {
                picture.data = picture
                    .data_as_raw_encoded()
                    .iter()
                    .flat_map(|value| {
                        let mut result = vec![];
                        for bit in 0..8 {
                            result.push(value >> bit & 0x01);
                        }
                        result
                    })
                    .collect();
            }
            PictureGraphicFormat::FourBit => {
                picture.data = picture
                    .data_as_raw_encoded()
                    .iter()
                    .flat_map(|value| {
                        let high = (value >> 4) & 0x0F;
                        let low = value & 0x0F;
                        vec![high, low]
                    })
                    .collect();
            }
            _ => {}
        },
    }
    picture.format = target;
    picture.options.data_code_type = DataCodeType::Raw;
}
