//! Read-only before/after comparisons using normal project edits and rendering.
use super::{checkerboard, fit_rect, object_preview_with_version};
use crate::EditorProject;
use ag_iso_stack::object_pool::{object::Object, ObjectId, ObjectPool};
use egui::{Rect, Vec2};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

/// Only objects whose own data changed belong in the comparison.
pub(crate) fn changed_objects(before: &ObjectPool, after: &ObjectPool) -> BTreeSet<u16> {
    let before: BTreeMap<_, _> = before
        .objects()
        .iter()
        .map(|object| (object.id().value(), object))
        .collect();
    let after: BTreeMap<_, _> = after
        .objects()
        .iter()
        .map(|object| (object.id().value(), object))
        .collect();
    before
        .keys()
        .chain(after.keys())
        .copied()
        .filter(|id| before.get(id) != after.get(id))
        .collect()
}

struct Comparison {
    before: ObjectPool,
    after: ObjectPool,
    changed: Vec<(ObjectId, String)>,
}

struct CachedReview {
    owner: egui::Id,
    revision: crate::cli::ProjectRevision,
    object_id_format: crate::ObjectIdFormat,
    result: Result<Comparison, String>,
}

fn cache_key() -> egui::Id {
    egui::Id::new("selection_change_preview")
}

/// A single bounded cache belongs to the currently pending selection. Reset it
/// whenever the shared picker opens, changes selection, confirms, or goes back.
pub(super) fn clear_cache(ctx: &egui::Context) {
    ctx.data_mut(|data| data.remove::<Arc<CachedReview>>(cache_key()));
}

fn prepare_comparison(
    project: &EditorProject,
    edit: impl FnOnce(&mut EditorProject) -> Result<(), String>,
) -> Result<Comparison, String> {
    let mut candidate = project.clone();
    edit(&mut candidate)?;
    let before = project
        .materialize()
        .map_err(|error| error.to_string())?
        .pool;
    let after = candidate
        .materialize()
        .map_err(|error| error.to_string())?
        .pool;
    let changed = changed_objects(&before, &after)
        .into_iter()
        .map(|value| {
            let id = ObjectId::new(value).unwrap();
            let object = after
                .object_by_id(id)
                .or_else(|| before.object_by_id(id))
                .unwrap();
            let label = candidate.object_display_name(object);
            (id, label)
        })
        .collect();
    Ok(Comparison {
        before,
        after,
        changed,
    })
}

/// Stage and validate once per pending selection/revision. Cloning the Arc on
/// subsequent frames does not clone pools or rerun the mutation/materializer.
pub fn review_change(
    ui: &mut egui::Ui,
    project: &EditorProject,
    edit: impl FnOnce(&mut EditorProject) -> Result<(), String>,
) -> bool {
    let owner = ui.next_auto_id();
    let revision = project.revision();
    let cached = ui
        .ctx()
        .data_mut(|data| data.get_temp::<Arc<CachedReview>>(cache_key()));
    let cached = if let Some(cached) = cached.filter(|cached| {
        cached.owner == owner
            && cached.revision == revision
            && cached.object_id_format == project.object_id_format()
    }) {
        cached
    } else {
        let cached = Arc::new(CachedReview {
            owner,
            revision,
            object_id_format: project.object_id_format(),
            result: prepare_comparison(project, edit),
        });
        ui.ctx()
            .data_mut(|data| data.insert_temp(cache_key(), cached.clone()));
        cached
    };
    match &cached.result {
        Ok(comparison) => {
            review_comparison(
                ui,
                comparison,
                project.object_id_format(),
                project.vt_version,
            );
            true
        }
        Err(error) => {
            crate::ui_style::error_text(ui, &format!("This change cannot be applied: {error}"));
            false
        }
    }
}

/// Reuse the end-of-frame operation path for inspector edits.
pub fn review_queued(
    ui: &mut egui::Ui,
    project: &EditorProject,
    edit: impl FnOnce(&EditorProject),
) -> bool {
    review_change(ui, project, |candidate| {
        edit(candidate);
        candidate.commit_queued("Preview selection");
        candidate.take_operation_error().map_or(Ok(()), Err)
    })
}

fn review_comparison(
    ui: &mut egui::Ui,
    comparison: &Comparison,
    format: crate::ObjectIdFormat,
    version: ag_iso_stack::object_pool::vt_version::VtVersion,
) {
    ui.separator();
    if comparison.changed.is_empty() {
        ui.label("This selection does not change the rendered objects.");
        return;
    }
    ui.strong("Changed objects — before / after");
    for (id, label) in &comparison.changed {
        draw_comparison(
            ui,
            &comparison.before,
            &comparison.after,
            *id,
            label,
            format,
            version,
        );
    }
}

fn dimensions(pool: &ObjectPool, object: &Object) -> Vec2 {
    let (w, h) = pool.content_size(object);
    if let Object::PictureGraphic(picture) = object {
        if w == 0 || h == 0 {
            return egui::vec2(picture.actual_width as f32, picture.actual_height as f32);
        }
    }
    egui::vec2(w as f32, h as f32)
}

fn draw_comparison(
    ui: &mut egui::Ui,
    before: &ObjectPool,
    after: &ObjectPool,
    id: ObjectId,
    label: &str,
    format: crate::ObjectIdFormat,
    version: ag_iso_stack::object_pool::vt_version::VtVersion,
) {
    let old = before.object_by_id(id);
    let new = after.object_by_id(id);
    ui.push_id(("impact", id.value()), |ui| {
        ui.label(label);
        let old_size = old.map_or(Vec2::ZERO, |object| dimensions(before, object));
        let new_size = new.map_or(Vec2::ZERO, |object| dimensions(after, object));
        let common_size = old_size.max(new_size).max(Vec2::splat(1.0));
        ui.columns(2, |columns| {
            for (ui, (label, pool, object, size)) in columns.iter_mut().zip([
                ("Before", before, old, old_size),
                ("After", after, new, new_size),
            ]) {
                ui.weak(label);
                let (rect, response) = ui.allocate_exact_size(
                    egui::vec2(ui.available_width().min(320.0), 170.0),
                    egui::Sense::hover(),
                );
                if ui.is_rect_visible(rect) {
                    checkerboard(ui.painter(), rect);
                }
                if let Some(object) = object {
                    // Both sides share one scale and origin, so a size change
                    // remains visible instead of independently filling each box.
                    let fitted = fit_rect(rect, common_size);
                    let scale = fitted.width() / common_size.x;
                    if ui.is_rect_visible(rect) {
                        object_preview_with_version(
                            ui,
                            Rect::from_min_size(fitted.min, size.max(Vec2::splat(1.0)) * scale),
                            pool,
                            object,
                            version,
                        );
                    }
                    super::hover_preview(response, |ui, rect| {
                        object_preview_with_version(ui, rect, pool, object, version)
                    });
                    ui.small(format!("{} × {}", size.x, size.y));
                    let refs = object
                        .referenced_objects()
                        .iter()
                        .map(|id| format!("#{}", format.format(id.value())))
                        .collect::<Vec<_>>()
                        .join(", ");
                    if !refs.is_empty() {
                        ui.small(format!("References: {refs}"));
                    }
                } else {
                    ui.painter().text(
                        rect.center(),
                        egui::Align2::CENTER_CENTER,
                        "Not present",
                        egui::FontId::proportional(14.0),
                        egui::Color32::DARK_GRAY,
                    );
                }
            }
        });
    });
}
