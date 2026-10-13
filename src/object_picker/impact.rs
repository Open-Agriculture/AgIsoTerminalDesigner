//! Read-only before/after comparisons using normal project edits and rendering.
use super::{checkerboard, fit_rect, object_preview};
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
    revision: crate::ai::ProjectRevision,
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
            let label = format!(
                "{} (#{} · {:?})",
                candidate.object_display_name(object),
                candidate.format_object_id(value),
                object.object_type()
            );
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
            review_comparison(ui, comparison, project.object_id_format());
            true
        }
        Err(error) => {
            ui.colored_label(
                egui::Color32::RED,
                format!("This change cannot be applied: {error}"),
            );
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

fn review_comparison(ui: &mut egui::Ui, comparison: &Comparison, format: crate::ObjectIdFormat) {
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
                        object_preview(
                            ui,
                            Rect::from_min_size(fitted.min, size.max(Vec2::splat(1.0)) * scale),
                            pool,
                            object,
                        );
                    }
                    super::hover_preview(response, |ui, rect| {
                        object_preview(ui, rect, pool, object)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphics::{integration::GraphicService, preview::picture_texture};
    use ag_iso_stack::object_pool::{ObjectRef, ObjectType};

    fn image_bytes(colour: [u8; 4]) -> Vec<u8> {
        let image = image::RgbaImage::from_pixel(12, 6, image::Rgba(colour));
        let mut bytes = std::io::Cursor::new(Vec::new());
        image.write_to(&mut bytes, image::ImageFormat::Png).unwrap();
        bytes.into_inner()
    }

    fn fixture() -> EditorProject {
        let mut pool = ObjectPool::default();
        for (id, kind) in [
            (1, ObjectType::DataMask),
            (10, ObjectType::Container),
            (20, ObjectType::Container),
            (30, ObjectType::Container),
            (120, ObjectType::PictureGraphic),
            (121, ObjectType::PictureGraphic),
        ] {
            let mut object = crate::default_object(kind);
            object.mut_id().set_value(id).unwrap();
            match &mut object {
                Object::DataMask(mask) => {
                    mask.object_refs = [10, 20, 30]
                        .into_iter()
                        .map(|id| ObjectRef {
                            id: ObjectId::new(id).unwrap(),
                            offset: Default::default(),
                        })
                        .collect()
                }
                Object::Container(container) => {
                    container.width = 100;
                    container.height = 80;
                    container.object_refs.push(ObjectRef {
                        id: ObjectId::new(if id == 30 { 121 } else { 120 }).unwrap(),
                        offset: Default::default(),
                    });
                }
                Object::PictureGraphic(picture) => {
                    crate::graphics::apply_image_bytes_to_picture_graphic(
                        picture,
                        &image_bytes([255, 0, 0, 255]),
                    )
                    .unwrap()
                }
                _ => {}
            }
            pool.add(object);
        }
        EditorProject::from(pool)
    }

    #[test]
    fn shared_graphic_compares_only_the_changed_graphic() {
        let project = fixture();
        let mut after = project.get_pool().clone();
        let Some(Object::PictureGraphic(picture)) =
            after.object_mut_by_id(ObjectId::new(120).unwrap())
        else {
            panic!()
        };
        crate::graphics::apply_image_bytes_to_picture_graphic(
            picture,
            &image_bytes([0, 255, 0, 255]),
        )
        .unwrap();
        assert_eq!(
            changed_objects(project.get_pool(), &after),
            BTreeSet::from([120])
        );
    }

    #[test]
    fn replacing_a_child_compares_only_the_changed_container() {
        let project = fixture();
        let mut after = project.get_pool().clone();
        let Some(Object::Container(container)) = after.object_mut_by_id(ObjectId::new(10).unwrap())
        else {
            panic!()
        };
        container.object_refs[0].id = ObjectId::new(121).unwrap();
        assert_eq!(
            changed_objects(project.get_pool(), &after),
            BTreeSet::from([10])
        );
    }

    #[test]
    fn review_uses_real_conversion_without_changing_live_project_or_history() {
        let mut project = fixture();
        let asset =
            GraphicService::upload(&mut project, "Green.png", image_bytes([0, 255, 0, 255]))
                .unwrap();
        let saved = project.save_project().unwrap();
        let revision = project.revision();
        let undo = project.undo_description();
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                assert!(review_change(ui, &project, |candidate| {
                    GraphicService::convert_asset_into_picture_graphic(candidate, asset, 120)
                        .map_err(|error| error.to_string())
                }));
                assert!(!review_change(ui, &project, |candidate| {
                    GraphicService::convert_asset_into_picture_graphic(candidate, asset, 999)
                        .map_err(|error| error.to_string())
                }));
            });
        });
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&project.save_project().unwrap()).unwrap(),
            serde_json::from_slice::<serde_json::Value>(&saved).unwrap()
        );
        assert_eq!(project.revision(), revision);
        assert_eq!(project.undo_description(), undo);
        assert!(project.graphic_library().assets.contains_key(&asset));
    }

    fn draw_cached_review(
        ctx: &egui::Context,
        project: &EditorProject,
        calls: &std::cell::Cell<usize>,
        fail: bool,
    ) {
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                assert_eq!(
                    review_change(ui, project, |_| {
                        calls.set(calls.get() + 1);
                        if fail {
                            Err("Invalid selection".into())
                        } else {
                            Ok(())
                        }
                    }),
                    !fail
                );
            });
        });
    }

    #[test]
    fn preparation_runs_once_until_project_or_selection_changes() {
        let mut project = fixture();
        let ctx = egui::Context::default();
        let calls = std::cell::Cell::new(0);
        for _ in 0..5 {
            draw_cached_review(&ctx, &project, &calls, false);
        }
        assert_eq!(calls.get(), 1);

        project.set_mask_size(project.mask_size + 1).unwrap();
        for _ in 0..3 {
            draw_cached_review(&ctx, &project, &calls, false);
        }
        assert_eq!(calls.get(), 2);

        super::super::open(&ctx, egui::Id::new("another_selection"));
        for _ in 0..3 {
            draw_cached_review(&ctx, &project, &calls, false);
        }
        assert_eq!(calls.get(), 3);
    }

    #[test]
    fn failed_preparation_is_cached_without_blocking_a_new_selection() {
        let project = fixture();
        let ctx = egui::Context::default();
        let calls = std::cell::Cell::new(0);
        for _ in 0..5 {
            draw_cached_review(&ctx, &project, &calls, true);
        }
        assert_eq!(calls.get(), 1);
        clear_cache(&ctx);
        draw_cached_review(&ctx, &project, &calls, false);
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn changed_objects_include_additions_and_removals() {
        let project = fixture();
        let mut after = project.get_pool().clone();
        after.remove(ObjectId::new(121).unwrap());
        let mut added = crate::default_object(ObjectType::PictureGraphic);
        added.mut_id().set_value(500).unwrap();
        after.add(added);
        assert_eq!(
            changed_objects(project.get_pool(), &after),
            BTreeSet::from([121, 500])
        );
    }

    #[test]
    fn before_after_graphic_textures_remain_cached_together() {
        let project = fixture();
        let pool = project.get_pool();
        let Some(Object::PictureGraphic(before)) = pool.object_by_id(ObjectId::new(120).unwrap())
        else {
            panic!()
        };
        let mut after = before.clone();
        crate::graphics::apply_image_bytes_to_picture_graphic(
            &mut after,
            &image_bytes([0, 255, 0, 255]),
        )
        .unwrap();
        let ctx = egui::Context::default();
        let first = picture_texture(&ctx, before, pool).unwrap();
        let second = picture_texture(&ctx, &after, pool).unwrap();
        assert_ne!(first.id(), second.id());
        for _ in 0..3 {
            assert_eq!(
                picture_texture(&ctx, before, pool).unwrap().id(),
                first.id()
            );
            assert_eq!(
                picture_texture(&ctx, &after, pool).unwrap().id(),
                second.id()
            );
        }
    }
}
