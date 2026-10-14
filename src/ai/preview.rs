//! Staged preview rendering. The UI path delegates to the application's normal renderer.
use super::design_analysis::{analyze_design, DesignAnalysisScope, DesignDiagnostic};
use super::snapshot::{active_mask, layout_instances};
use super::{AiDesignState, AiProposal};
use ag_iso_stack::object_pool::{ObjectId, ObjectPool};
use eframe::egui;
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
    pub proposal_id: String,
    pub width: u32,
    pub height: u32,
    pub image: AiImage,
    pub diagnostics: Vec<DesignDiagnostic>,
    pub mask_id: u16,
    pub colour_summary: serde_json::Value,
}
pub type AiPreview = RenderedPreview;
#[derive(Debug, Clone)]
pub enum PreviewError {
    NoActiveMask,
    MissingMask,
    Encode(String),
    Analysis(String),
}

fn candidate<'a>(
    _state: &'a AiDesignState,
    proposal: &'a AiProposal,
) -> (
    &'a ObjectPool,
    &'a std::collections::HashMap<ObjectId, crate::ObjectInfo>,
) {
    (&proposal.prepared.pool, &proposal.prepared.object_info)
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

/// Draw a staged mask inside an egui UI. This is the preview the user sees and
/// is exactly the normal `RenderableObject` implementation.
pub fn render_preview_ui(
    ui: &mut egui::Ui,
    state: &AiDesignState,
    proposal: &AiProposal,
) -> Result<(), PreviewError> {
    let (pool, _) = candidate(state, proposal);
    let candidate_state = proposal_state(state, proposal);
    let id = active_mask(pool).ok_or(PreviewError::NoActiveMask)?;
    let mask = pool.object_by_id(id).ok_or(PreviewError::MissingMask)?;
    let (w, h) = dimensions(candidate_state.mask_size);
    ui.allocate_ui(egui::vec2(w as f32, h as f32), |ui| {
        crate::object_rendering::render_mask_with_version(
            ui,
            pool,
            mask,
            candidate_state.vt_version,
        )
    });
    Ok(())
}

/// Produce a bounded PNG transport image. Object geometry is sampled from the
/// same pool/layout consumed by the normal renderer; the interactive UI preview
/// above remains the authoritative visual rendering path.
pub fn render_proposal_preview(
    state: &AiDesignState,
    proposal: &AiProposal,
) -> Result<RenderedPreview, PreviewError> {
    let (pool, info) = candidate(state, proposal);
    let candidate_state = proposal_state(state, proposal);
    render_design_preview_inner(
        &candidate_state,
        pool,
        info,
        Some(proposal.proposal_id.clone()),
        None,
    )
}

fn proposal_state(state: &AiDesignState, proposal: &AiProposal) -> AiDesignState {
    let mut candidate = state.clone();
    candidate.pool = proposal.prepared.pool.clone();
    candidate.object_info = proposal.prepared.object_info.clone();
    candidate.canonical_pool = Some(proposal.canonical_pool.clone());
    candidate.components = proposal.components.clone();
    candidate.graphics = proposal.graphics.clone();
    candidate.soft_key_layout = proposal.soft_key_layout;
    if let Some(content) = &proposal.candidate_content {
        candidate.mask_size = content.mask_size;
        candidate.soft_key_size = content.soft_key_size;
        candidate.soft_key_layout = content.soft_key_layout;
        candidate.vt_version = content.vt_version;
        candidate.object_id_ranges = content.object_id_ranges.clone();
    }
    candidate
}

/// Produce the same bounded PNG transport preview for an explicitly selected
/// design state. When `mask_id` is omitted, the state's active mask is used.
pub fn render_design_preview(
    state: &AiDesignState,
    mask_id: Option<u16>,
) -> Result<RenderedPreview, PreviewError> {
    render_design_preview_inner(state, &state.pool, &state.object_info, None, mask_id)
}

fn render_design_preview_inner(
    state: &AiDesignState,
    pool: &ObjectPool,
    info: &std::collections::HashMap<ObjectId, crate::ObjectInfo>,
    proposal_id: Option<String>,
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
    let analysis_state = AiDesignState {
        pool: pool.clone(),
        object_info: info.clone(),
        ..state.clone()
    };
    let diagnostics = analyze_design(&analysis_state, None, DesignAnalysisScope::ActiveMask)
        .map_err(|e| PreviewError::Analysis(format!("{e:?}")))?
        .diagnostics;
    Ok(RenderedPreview {
        proposal_id: proposal_id.unwrap_or_default(),
        width,
        height,
        image: AiImage {
            mime_type: "image/png".into(),
            bytes,
        },
        diagnostics,
        mask_id: mask.value(),
        colour_summary: super::visual_diagnostics::colour_summary(pool),
    })
}
