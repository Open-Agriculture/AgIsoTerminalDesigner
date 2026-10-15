//! Bounded schematic previews sampled from the project layout.
use super::snapshot::{active_mask, layout_instances};
use super::AiDesignState;
use ag_iso_stack::object_pool::{ObjectId, ObjectPool};
use image::{ImageFormat, Rgba, RgbaImage};
use std::io::Cursor;

pub const MAX_AI_PREVIEW_WIDTH: u32 = 800;
pub const MAX_AI_PREVIEW_HEIGHT: u32 = 600;
#[derive(Debug, Clone)]
pub struct AiImage {
    pub mime_type: String,
    pub bytes: Vec<u8>,
}
#[derive(Debug, Clone)]
pub struct RenderedPreview {
    pub width: u32,
    pub height: u32,
    pub image: AiImage,
    pub mask_id: u16,
}
#[derive(Debug, Clone)]
pub enum PreviewError {
    NoActiveMask,
    MissingMask,
    Encode(String),
}

fn dimensions(size: u16) -> (u32, u32) {
    let s = u32::from(size);
    let scale = (MAX_AI_PREVIEW_WIDTH as f32 / s as f32)
        .min(MAX_AI_PREVIEW_HEIGHT as f32 / s as f32)
        .min(1.0);
    (
        (s as f32 * scale).max(1.) as u32,
        (s as f32 * scale).max(1.) as u32,
    )
}

/// Produce the same bounded PNG transport preview for an explicitly selected
/// design state. When `mask_id` is omitted, the state's active mask is used.
pub fn render_design_preview(
    state: &AiDesignState,
    mask_id: Option<u16>,
) -> Result<RenderedPreview, PreviewError> {
    render_design_preview_inner(state, &state.pool, &state.object_info, mask_id)
}

fn render_design_preview_inner(
    state: &AiDesignState,
    pool: &ObjectPool,
    info: &std::collections::HashMap<ObjectId, crate::ObjectInfo>,
    requested_mask: Option<u16>,
) -> Result<RenderedPreview, PreviewError> {
    let mask = match requested_mask {
        Some(id) => ObjectId::new(id).map_err(|_| PreviewError::MissingMask)?,
        None => active_mask(pool).ok_or(PreviewError::NoActiveMask)?,
    };
    if pool.object_by_id(mask).is_none() {
        return Err(PreviewError::MissingMask);
    };
    let (width, height) = dimensions(state.mask_size);
    let mut pixels = RgbaImage::from_pixel(width, height, Rgba([245, 245, 245, 255]));
    let sx = width as f32 / state.mask_size.max(1) as f32;
    let sy = height as f32 / state.mask_size.max(1) as f32;
    for item in layout_instances(pool, info, Some(mask), super::snapshot::MAX_SNAPSHOT_DEPTH) {
        let (Some(iw), Some(ih)) = (item.width, item.height) else {
            continue;
        };
        let x = (item.x as f32 * sx).max(0.) as u32;
        let y = (item.y as f32 * sy).max(0.) as u32;
        let rw = (f32::from(iw) * sx).max(1.) as u32;
        let rh = (f32::from(ih) * sy).max(1.) as u32;
        for px in x..x.saturating_add(rw).min(width) {
            for py in y..y.saturating_add(rh).min(height) {
                if px == x
                    || py == y
                    || px + 1 == x.saturating_add(rw).min(width)
                    || py + 1 == y.saturating_add(rh).min(height)
                {
                    pixels.put_pixel(px, py, Rgba([50, 90, 130, 255]));
                }
            }
        }
    }
    let mut bytes = Vec::new();
    pixels
        .write_to(&mut Cursor::new(&mut bytes), ImageFormat::Png)
        .map_err(|e| PreviewError::Encode(e.to_string()))?;
    Ok(RenderedPreview {
        width,
        height,
        image: AiImage {
            mime_type: "image/png".into(),
            bytes,
        },
        mask_id: mask.value(),
    })
}
