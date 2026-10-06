//! Deterministic visual quality checks.
use super::snapshot::{active_mask, layout_instances, summary, LayoutInstance, ObjectSummary};
use super::{AiDesignState, AiProposal};
use ag_iso_stack::object_pool::ObjectId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

pub const ALIGNMENT_TOLERANCE: i32 = 4;
pub const SPACING_TOLERANCE: i32 = 4;
pub const MIN_TOUCH_TARGET: u16 = 20;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum DesignDiagnosticSeverity {
    Error,
    Warning,
    Info,
}
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct DiagnosticRegion {
    pub x: i32,
    pub y: i32,
    pub width: u16,
    pub height: u16,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DesignDiagnostic {
    pub severity: DesignDiagnosticSeverity,
    pub code: String,
    pub message: String,
    pub objects: Vec<ObjectSummary>,
    pub region: Option<DiagnosticRegion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<serde_json::Value>,
}
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum DesignAnalysisScope {
    ActiveMask,
    Proposal,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DesignLayoutSummary {
    pub root: Option<ObjectSummary>,
    pub instance_count: usize,
    pub width: u16,
    pub height: u16,
}
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DesignAnalysis {
    pub diagnostics: Vec<DesignDiagnostic>,
    pub layout_summary: DesignLayoutSummary,
}
#[derive(Debug, Clone)]
pub enum DesignAnalysisError {
    NoActiveMask,
}

fn candidate<'a>(
    state: &'a AiDesignState,
    proposal: Option<&'a AiProposal>,
) -> (
    ag_iso_stack::object_pool::ObjectPool,
    HashMap<ObjectId, crate::ObjectInfo>,
) {
    match proposal {
        Some(p) => (p.prepared.pool.clone(), p.prepared.object_info.clone()),
        None => (state.pool.clone(), state.object_info.clone()),
    }
}
fn rect(instance: &LayoutInstance) -> Option<DiagnosticRegion> {
    Some(DiagnosticRegion {
        x: instance.x,
        y: instance.y,
        width: instance.width?,
        height: instance.height?,
    })
}
fn intersection(a: &DiagnosticRegion, b: &DiagnosticRegion) -> Option<DiagnosticRegion> {
    let x = a.x.max(b.x);
    let y = a.y.max(b.y);
    let right = (a.x + i32::from(a.width)).min(b.x + i32::from(b.width));
    let bottom = (a.y + i32::from(a.height)).min(b.y + i32::from(b.height));
    (right > x && bottom > y).then_some(DiagnosticRegion {
        x,
        y,
        width: (right - x) as u16,
        height: (bottom - y) as u16,
    })
}
fn is_interactive(t: &str) -> bool {
    matches!(
        t,
        "Button" | "Key" | "InputBoolean" | "InputString" | "InputNumber" | "InputList"
    )
}

pub fn analyze_design(
    state: &AiDesignState,
    proposal: Option<&AiProposal>,
    scope: DesignAnalysisScope,
) -> Result<DesignAnalysis, DesignAnalysisError> {
    analyze_design_with_thresholds(state, proposal, scope, Default::default())
}
pub fn analyze_design_with_thresholds(
    state: &AiDesignState,
    proposal: Option<&AiProposal>,
    _scope: DesignAnalysisScope,
    thresholds: super::visual_diagnostics::ContrastThresholds,
) -> Result<DesignAnalysis, DesignAnalysisError> {
    let (pool, info) = candidate(state, proposal);
    let root_id = active_mask(&pool).ok_or(DesignAnalysisError::NoActiveMask)?;
    let root = summary(&pool, &info, root_id).ok_or(DesignAnalysisError::NoActiveMask)?;
    let layout = layout_instances(
        &pool,
        &info,
        Some(root_id),
        super::snapshot::MAX_SNAPSHOT_DEPTH,
    );
    let mut diagnostics =
        super::visual_diagnostics::contrast_diagnostics(&pool, &info, &layout, thresholds);
    let bounds: Vec<_> = layout
        .iter()
        .filter_map(|v| rect(v).map(|r| (v, r)))
        .collect();
    // Only siblings are checked: parent/child layering is an intentional VT pattern.
    for (index, (a, ar)) in bounds.iter().enumerate() {
        for (b, br) in bounds.iter().skip(index + 1) {
            if a.parent.id != b.parent.id {
                continue;
            }
            if let Some(region) = intersection(ar, br) {
                diagnostics.push(DesignDiagnostic {
                    context: None,
                    severity: DesignDiagnosticSeverity::Warning,
                    code: "OVERLAP".into(),
                    message: format!(
                        "{} overlaps {} by {} px",
                        a.object.name,
                        b.object.name,
                        u32::from(region.width) * u32::from(region.height)
                    ),
                    objects: vec![a.object.clone(), b.object.clone()],
                    region: Some(region),
                });
            }
        }
    }
    for (item, r) in &bounds {
        let parent_rect = if item.parent.id == root.id {
            Some(DiagnosticRegion {
                x: 0,
                y: 0,
                width: state.mask_size,
                height: state.mask_size,
            })
        } else {
            bounds
                .iter()
                .find(|(p, _)| p.object.id == item.parent.id)
                .map(|(_, r)| r.clone())
        };
        if let Some(parent) = parent_rect {
            if r.x < parent.x
                || r.y < parent.y
                || r.x + i32::from(r.width) > parent.x + i32::from(parent.width)
                || r.y + i32::from(r.height) > parent.y + i32::from(parent.height)
            {
                diagnostics.push(DesignDiagnostic {
                    context: None,
                    severity: DesignDiagnosticSeverity::Warning,
                    code: "OUTSIDE_PARENT_BOUNDS".into(),
                    message: format!("{} extends outside {}", item.object.name, item.parent.name),
                    objects: vec![item.object.clone(), item.parent.clone()],
                    region: intersection(r, &parent).or(Some(r.clone())),
                });
            }
        }
        if is_interactive(&item.object.object_type)
            && (r.width < MIN_TOUCH_TARGET || r.height < MIN_TOUCH_TARGET)
        {
            diagnostics.push(DesignDiagnostic {
                context: None,
                severity: DesignDiagnosticSeverity::Warning,
                code: "SMALL_TOUCH_TARGET".into(),
                message: format!(
                    "{} is smaller than the {} px design touch-target threshold",
                    item.object.name, MIN_TOUCH_TARGET
                ),
                objects: vec![item.object.clone()],
                region: Some(r.clone()),
            });
        }
    }
    // Conservative alignment: a sibling pair whose left edges are close but not exact.
    for (index, (a, ar)) in bounds.iter().enumerate() {
        for (b, br) in bounds.iter().skip(index + 1) {
            if a.parent.id == b.parent.id {
                let d = (ar.x - br.x).abs();
                if d > 0 && d <= ALIGNMENT_TOLERANCE {
                    diagnostics.push(DesignDiagnostic {
                        context: None,
                        severity: DesignDiagnosticSeverity::Info,
                        code: "INCONSISTENT_ALIGNMENT".into(),
                        message: format!(
                            "{} and {} are nearly left-aligned",
                            a.object.name, b.object.name
                        ),
                        objects: vec![a.object.clone(), b.object.clone()],
                        region: None,
                    });
                }
            }
        }
    }
    // Stable row/column spacing check for three or more sibling objects sharing a near x coordinate.
    let mut by_parent: HashMap<u16, Vec<(&LayoutInstance, &DiagnosticRegion)>> = HashMap::new();
    for (i, r) in &bounds {
        by_parent.entry(i.parent.id).or_default().push((i, r));
    }
    for siblings in by_parent.values_mut() {
        siblings.sort_by_key(|(_, r)| r.y);
        for triple in siblings.windows(3) {
            let (a, ar) = triple[0];
            let (b, br) = triple[1];
            let (c, cr) = triple[2];
            if (ar.x - br.x).abs() <= ALIGNMENT_TOLERANCE
                && (br.x - cr.x).abs() <= ALIGNMENT_TOLERANCE
            {
                let g1 = br.y - (ar.y + i32::from(ar.height));
                let g2 = cr.y - (br.y + i32::from(br.height));
                if (g1 - g2).abs() > SPACING_TOLERANCE {
                    diagnostics.push(DesignDiagnostic {
                        context: None,
                        severity: DesignDiagnosticSeverity::Info,
                        code: "INCONSISTENT_SPACING".into(),
                        message: format!("Vertical spacing near {} is inconsistent", b.object.name),
                        objects: vec![a.object.clone(), b.object.clone(), c.object.clone()],
                        region: None,
                    });
                }
            }
        }
    }
    let source = proposal.map(|p| &p.components).unwrap_or(&state.components);
    for diagnostic in &mut diagnostics {
        let sources: Vec<_> = diagnostic
            .objects
            .iter()
            .filter_map(|o| super::tools::source_context(source, o.id))
            .collect();
        if !sources.is_empty() {
            let mut context = diagnostic
                .context
                .take()
                .unwrap_or_else(|| serde_json::json!({}));
            context["component_sources"] = serde_json::json!(sources);
            diagnostic.context = Some(context);
        }
    }
    Ok(DesignAnalysis {
        diagnostics,
        layout_summary: DesignLayoutSummary {
            root: Some(root),
            instance_count: layout.len(),
            width: state.mask_size,
            height: state.mask_size,
        },
    })
}
