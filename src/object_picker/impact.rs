//! Read-only before/after comparisons using normal project edits and rendering.
use super::{checkerboard, fit_rect, object_preview};
use crate::EditorProject;
use ag_iso_stack::object_pool::{object::Object, ObjectId, ObjectPool};
use egui::{Rect, Vec2};
use std::collections::BTreeSet;

/// Returns changed objects and their direct users in either version, exactly once.
pub(crate) fn affected_objects(
    before: &ObjectPool,
    after: &ObjectPool,
) -> (BTreeSet<u16>, BTreeSet<u16>) {
    let changed: BTreeSet<_> = before
        .objects()
        .iter()
        .chain(after.objects())
        .filter_map(|object| {
            let id = object.id();
            (before.object_by_id(id) != after.object_by_id(id)).then_some(id.value())
        })
        .collect();
    let parents = changed
        .iter()
        .flat_map(|id| {
            crate::pool_validation::get_referencing_objects(before, ObjectId::new(*id).unwrap())
                .into_iter()
                .chain(crate::pool_validation::get_referencing_objects(
                    after,
                    ObjectId::new(*id).unwrap(),
                ))
        })
        .map(|id| id.value())
        .filter(|id| !changed.contains(id))
        .collect();
    (changed, parents)
}

/// Run the real mutation on a disposable project. The live pool, selection and
/// history remain untouched until the caller receives a confirmed selection.
pub fn review_change(
    ui: &mut egui::Ui,
    project: &EditorProject,
    edit: impl FnOnce(&mut EditorProject) -> Result<(), String>,
) -> bool {
    let mut candidate = project.clone();
    let result = edit(&mut candidate).and_then(|()| {
        let before = project.materialize().map_err(|error| error.to_string())?;
        let after = candidate.materialize().map_err(|error| error.to_string())?;
        Ok((before.pool, after.pool))
    });
    match result {
        Ok((before, after)) => {
            review_pools(ui, &candidate, &before, &after);
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

fn review_pools(
    ui: &mut egui::Ui,
    project: &EditorProject,
    before: &ObjectPool,
    after: &ObjectPool,
) {
    let (changed, parents) = affected_objects(before, after);
    ui.separator();
    if changed.is_empty() {
        ui.label("This selection does not change the rendered objects.");
        return;
    }
    ui.strong("Changed objects — before / after");
    for id in changed {
        comparison(ui, project, before, after, ObjectId::new(id).unwrap());
    }
    ui.separator();
    ui.strong(format!(
        "Direct parent objects ({}) — before / after",
        parents.len()
    ));
    if parents.is_empty() {
        ui.label("No other objects directly use these changed objects.");
    }
    for id in parents {
        comparison(ui, project, before, after, ObjectId::new(id).unwrap());
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

fn comparison(
    ui: &mut egui::Ui,
    project: &EditorProject,
    before: &ObjectPool,
    after: &ObjectPool,
    id: ObjectId,
) {
    let old = before.object_by_id(id);
    let new = after.object_by_id(id);
    let Some(object) = new.or(old) else {
        return;
    };
    ui.push_id(("impact", id.value()), |ui| {
        ui.label(format!(
            "{} (#{} · {:?})",
            project.get_object_info(object).get_name(object),
            id.value(),
            object.object_type()
        ));
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
                checkerboard(ui.painter(), rect);
                if let Some(object) = object {
                    // Both sides share one scale and origin, so a size change
                    // remains visible instead of independently filling each box.
                    let fitted = fit_rect(rect, common_size);
                    let scale = fitted.width() / common_size.x;
                    object_preview(
                        ui,
                        Rect::from_min_size(fitted.min, size.max(Vec2::splat(1.0)) * scale),
                        pool,
                        object,
                    );
                    super::hover_preview(response, |ui, rect| {
                        object_preview(ui, rect, pool, object)
                    });
                    ui.small(format!("{} × {}", size.x, size.y));
                    let refs = object
                        .referenced_objects()
                        .iter()
                        .map(|id| format!("#{}", id.value()))
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
    fn shared_graphic_lists_every_direct_parent_without_grandparents_or_unrelated_objects() {
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
        let (changed, parents) = affected_objects(project.get_pool(), &after);
        assert_eq!(changed, BTreeSet::from([120]));
        assert_eq!(parents, BTreeSet::from([10, 20]));
    }

    #[test]
    fn replacing_a_child_compares_the_changed_container_and_its_direct_mask() {
        let project = fixture();
        let mut after = project.get_pool().clone();
        let Some(Object::Container(container)) = after.object_mut_by_id(ObjectId::new(10).unwrap())
        else {
            panic!()
        };
        container.object_refs[0].id = ObjectId::new(121).unwrap();
        let (changed, parents) = affected_objects(project.get_pool(), &after);
        assert_eq!(changed, BTreeSet::from([10]));
        assert_eq!(parents, BTreeSet::from([1]));
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
