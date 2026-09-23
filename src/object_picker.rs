//! Shared searchable selection window and aspect-preserving preview rows.
use crate::{EditorProject, RenderableObject};
use ag_iso_stack::object_pool::{
    object::Object, object_attributes::Point, NullableObjectId, ObjectId, ObjectPool,
};
use egui::{self, Rect, Vec2};

const ACTIVE: egui::Id = egui::Id::NULL;

pub fn open(ctx: &egui::Context, id: egui::Id) {
    ctx.data_mut(|data| {
        data.insert_temp(ACTIVE.with("selection_window"), id);
        data.insert_temp(id.with("query"), String::new());
        data.insert_temp(id.with("focus"), true);
    });
}

pub fn is_open(ctx: &egui::Context, id: egui::Id) -> bool {
    ctx.data(|data| data.get_temp::<egui::Id>(ACTIVE.with("selection_window"))) == Some(id)
}

pub fn matches(name: &str, id: impl std::fmt::Display, query: &str) -> bool {
    let query = query.trim().to_lowercase();
    name.to_lowercase().contains(&query)
        || id
            .to_string()
            .to_lowercase()
            .contains(query.trim_start_matches('#'))
}

/// All callers share window behavior, search focus, scrolling, and dismissal.
pub fn window<T>(
    ctx: &egui::Context,
    id: egui::Id,
    title: &str,
    contents: impl FnOnce(&mut egui::Ui, &str) -> Option<T>,
) -> Option<T> {
    if !is_open(ctx, id) {
        return None;
    }
    let mut query = ctx.data_mut(|data| {
        data.get_temp::<String>(id.with("query"))
            .unwrap_or_default()
    });
    let focus = ctx
        .data_mut(|data| data.remove_temp::<bool>(id.with("focus")))
        .unwrap_or(false);
    let mut open =
        !ctx.input_mut(|input| input.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    let mut selected = None;
    egui::Window::new(title)
        .id(id)
        .open(&mut open)
        .collapsible(false)
        .default_size([460.0, 480.0])
        .show(ctx, |ui| {
            let search = ui.add(
                egui::TextEdit::singleline(&mut query)
                    .hint_text("Search by name or ID…")
                    .desired_width(f32::INFINITY),
            );
            if focus {
                search.request_focus();
            }
            ui.separator();
            egui::ScrollArea::vertical()
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    selected = contents(ui, &query);
                });
        });
    ctx.data_mut(|data| {
        data.insert_temp(id.with("query"), query);
        if !open || selected.is_some() {
            data.remove::<egui::Id>(ACTIVE.with("selection_window"));
        }
    });
    selected
}

pub fn fit_rect(bounds: Rect, size: Vec2) -> Rect {
    let size = size.max(Vec2::splat(1.0));
    let scale = (bounds.width() / size.x).min(bounds.height() / size.y);
    Rect::from_center_size(bounds.center(), size * scale)
}

pub fn checkerboard(painter: &egui::Painter, rect: Rect) {
    const TILE: f32 = 8.0;
    for row in 0..(rect.height() / TILE).ceil() as usize {
        for column in 0..(rect.width() / TILE).ceil() as usize {
            let tile = Rect::from_min_size(
                rect.min + egui::vec2(column as f32 * TILE, row as f32 * TILE),
                Vec2::splat(TILE),
            )
            .intersect(rect);
            painter.rect_filled(
                tile,
                0.0,
                egui::Color32::from_gray(if (row + column) % 2 == 0 { 190 } else { 235 }),
            );
        }
    }
}

pub fn texture(ui: &mut egui::Ui, rect: Rect, texture: &egui::TextureHandle) {
    ui.painter().image(
        texture.id(),
        fit_rect(rect, texture.size_vec2()),
        Rect::from_min_max(egui::Pos2::ZERO, egui::pos2(1.0, 1.0)),
        egui::Color32::WHITE,
    );
}

/// Read-only miniature; render at natural size, then uniformly transform its shapes.
pub fn object_preview(ui: &mut egui::Ui, rect: Rect, pool: &ObjectPool, object: &Object) {
    if let Object::PictureGraphic(picture) = object {
        if let Some(image) = crate::graphics::preview::picture_texture(ui.ctx(), picture, pool) {
            texture(ui, rect, &image);
        }
        return;
    }
    let (width, height) = pool.content_size(object);
    if width == 0 || height == 0 {
        ui.painter().text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "—",
            egui::FontId::proportional(18.0),
            egui::Color32::DARK_GRAY,
        );
        return;
    }
    let size = egui::vec2(width as f32, height as f32);
    let fitted = fit_rect(rect, size);
    let transform = egui::emath::TSTransform::new(fitted.min.to_vec2(), fitted.width() / size.x);
    // A detached child Ui prevents natural dimensions from expanding the row.
    let mut preview = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(Rect::from_min_size(egui::Pos2::ZERO, size))
            .layout(egui::Layout::top_down(egui::Align::Min))
            .sense(egui::Sense::hover()),
    );
    preview.set_clip_rect(transform.inverse().mul_rect(rect.intersect(ui.clip_rect())));
    preview.with_visual_transform(transform, |ui| {
        object.render(ui, pool, Point::default());
    });
}

pub fn preview(ui: &mut egui::Ui, paint: impl Fn(&mut egui::Ui, Rect)) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(64.0, 48.0), egui::Sense::click());
    if ui.is_rect_visible(rect) {
        checkerboard(ui.painter(), rect);
        paint(ui, rect);
    }
    hover_preview(response, paint)
}

pub fn hover_preview(
    response: egui::Response,
    paint: impl Fn(&mut egui::Ui, Rect),
) -> egui::Response {
    response.on_hover_ui(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(280.0, 220.0), egui::Sense::hover());
        checkerboard(ui.painter(), rect);
        paint(ui, rect);
    })
}

pub fn row(
    ui: &mut egui::Ui,
    name: &str,
    detail: &str,
    selected: bool,
    paint: impl Fn(&mut egui::Ui, Rect),
) -> bool {
    ui.horizontal(|ui| {
        let image = preview(ui, paint);
        let text = ui
            .vertical(|ui| {
                let response = ui.selectable_label(selected, name);
                ui.weak(detail);
                response
            })
            .inner;
        image.clicked() || text.clicked()
    })
    .inner
}

pub fn objects(
    ui: &mut egui::Ui,
    project: &EditorProject,
    query: &str,
    selected: Option<ObjectId>,
    allow_none: bool,
    eligible: impl Fn(&Object) -> bool,
) -> Option<NullableObjectId> {
    let mut result = None;
    if allow_none
        && matches("None", "", query)
        && ui.selectable_label(selected.is_none(), "None").clicked()
    {
        result = Some(NullableObjectId::NULL);
    }
    let mut count = 0;
    for object in project.get_pool().objects() {
        let name = project.get_object_info(object).get_name(object);
        if !eligible(object) || !matches(&name, object.id().value(), query) {
            continue;
        }
        count += 1;
        ui.push_id(object.id().value(), |ui| {
            if row(
                ui,
                &name,
                &format!("#{} · {:?}", object.id().value(), object.object_type()),
                selected == Some(object.id()),
                |ui, rect| object_preview(ui, rect, project.get_pool(), object),
            ) {
                result = Some(NullableObjectId(Some(object.id())));
            }
        });
    }
    if count == 0 {
        ui.label("No matching objects.");
    }
    result
}

pub fn object_window(
    ctx: &egui::Context,
    id: egui::Id,
    title: &str,
    project: &EditorProject,
    selected: Option<ObjectId>,
    allow_none: bool,
    eligible: impl Fn(&Object) -> bool,
) -> Option<NullableObjectId> {
    window(ctx, id, title, |ui, query| {
        objects(ui, project, query, selected, allow_none, eligible)
    })
}

pub fn button(
    ui: &mut egui::Ui,
    salt: impl std::hash::Hash,
    label: &str,
    project: &EditorProject,
    selected: Option<ObjectId>,
    allow_none: bool,
    eligible: impl Fn(&Object) -> bool,
) -> Option<NullableObjectId> {
    let id = ui.make_persistent_id(salt);
    if ui.button(label).clicked() {
        open(ui.ctx(), id);
    }
    object_window(
        ui.ctx(),
        id,
        "Select object",
        project,
        selected,
        allow_none,
        eligible,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use ag_iso_stack::object_pool::ObjectType;

    fn fixture() -> EditorProject {
        let mut pool = ObjectPool::default();
        for (id, kind) in [
            (12, ObjectType::Button),
            (120, ObjectType::PictureGraphic),
            (300, ObjectType::Macro),
        ] {
            let mut object = crate::default_object(kind);
            object.mut_id().set_value(id).unwrap();
            if let Object::Button(button) = &mut object {
                button.width = 400;
                button.height = 100;
            }
            pool.add(object);
        }
        let mut project = EditorProject::from(pool);
        assert!(project.rename_object(ObjectId::new(12).unwrap(), "Wide Button".into()));
        assert!(project.rename_object(ObjectId::new(120).unwrap(), "Warning".into()));
        project
    }

    fn texts(shapes: &[egui::epaint::ClippedShape]) -> String {
        fn collect(shape: &egui::Shape, out: &mut String) {
            match shape {
                egui::Shape::Text(text) => {
                    out.push_str(text.galley.text());
                    out.push('\n');
                }
                egui::Shape::Vec(shapes) => {
                    for shape in shapes {
                        collect(shape, out);
                    }
                }
                _ => {}
            }
        }
        let mut out = String::new();
        for shape in shapes {
            collect(&shape.shape, &mut out);
        }
        out
    }

    #[test]
    fn name_and_id_search_respects_caller_eligibility() {
        let project = fixture();
        let ctx = egui::Context::default();
        for (query, pictures_only, expected, excluded) in [
            (" wArNiNg ", false, "Warning", "Wide Button"),
            ("#12", false, "Wide Button", "Macro"),
            ("12", true, "Warning", "Wide Button"),
            ("missing", true, "No matching objects", "Warning"),
        ] {
            let output = ctx.run(egui::RawInput::default(), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    objects(ui, &project, query, None, false, |object| {
                        !pictures_only || object.object_type() == ObjectType::PictureGraphic
                    });
                });
            });
            let text = texts(&output.shapes);
            assert!(text.contains(expected), "{text}");
            assert!(!text.contains(excluded), "{text}");
        }
    }

    #[test]
    fn previews_keep_aspect_ratio_and_do_not_expand_layout() {
        let bounds = Rect::from_min_size(egui::pos2(10.0, 20.0), egui::vec2(64.0, 48.0));
        for size in [egui::vec2(400.0, 100.0), egui::vec2(100.0, 400.0)] {
            let fitted = fit_rect(bounds, size);
            assert!(bounds.contains_rect(fitted));
            assert_eq!(fitted.center(), bounds.center());
            assert!((fitted.aspect_ratio() - size.x / size.y).abs() < 0.001);
        }
        let project = fixture();
        let before = project.save_project().unwrap();
        let ctx = egui::Context::default();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let object = project
                    .get_pool()
                    .object_by_id(ObjectId::new(12).unwrap())
                    .unwrap();
                let small = Rect::from_min_size(ui.cursor().min, egui::vec2(80.0, 60.0));
                let row = ui.scope_builder(egui::UiBuilder::new().max_rect(small), |ui| {
                    let response = preview(ui, |ui, rect| {
                        object_preview(ui, rect, project.get_pool(), object)
                    });
                    assert_eq!(response.rect.size(), egui::vec2(64.0, 48.0));
                });
                assert!(row.response.rect.width() <= 80.0);
            });
        });
        assert_eq!(project.save_project().unwrap(), before);
    }

    #[test]
    fn hover_draws_a_larger_preview() {
        let ctx = egui::Context::default();
        ctx.style_mut(|style| {
            style.interaction.tooltip_delay = 0.0;
            style.interaction.tooltip_grace_time = 0.0;
        });
        let sizes = std::cell::RefCell::new(Vec::new());
        for frame in 0..4 {
            let _ = ctx.run(
                egui::RawInput {
                    time: Some(frame as f64),
                    events: vec![egui::Event::PointerMoved(egui::pos2(25.0, 25.0))],
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        preview(ui, |_, rect| sizes.borrow_mut().push(rect.size()));
                    });
                },
            );
        }
        let sizes = sizes.borrow();
        assert!(sizes.contains(&egui::vec2(64.0, 48.0)));
        assert!(sizes.contains(&egui::vec2(280.0, 220.0)), "{sizes:?}");
    }

    #[test]
    fn clicking_a_filtered_preview_selects_its_object() {
        let project = fixture();
        let ctx = egui::Context::default();
        let mut selected = None;
        for pressed in [None, Some(true), Some(false)] {
            let mut events = vec![egui::Event::PointerMoved(egui::pos2(25.0, 25.0))];
            if let Some(pressed) = pressed {
                events.push(egui::Event::PointerButton {
                    pos: egui::pos2(25.0, 25.0),
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            let _ = ctx.run(
                egui::RawInput {
                    events,
                    ..Default::default()
                },
                |ctx| {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        if let Some(choice) = objects(ui, &project, "#120", None, false, |object| {
                            object.object_type() == ObjectType::PictureGraphic
                        }) {
                            selected = choice.0;
                        }
                    });
                },
            );
        }
        assert_eq!(selected, ObjectId::new(120).ok());
    }

    #[test]
    fn opening_another_picker_resets_search_and_selection_closes_it() {
        let ctx = egui::Context::default();
        let first = egui::Id::new("first");
        let second = egui::Id::new("second");
        open(&ctx, first);
        ctx.data_mut(|data| data.insert_temp(first.with("query"), "old search".to_owned()));
        open(&ctx, second);
        assert!(!is_open(&ctx, first));
        open(&ctx, first);
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            let choice = window(ctx, first, "Choose", |_, query| {
                assert!(query.is_empty());
                Some(12)
            });
            assert_eq!(choice, Some(12));
        });
        assert!(!is_open(&ctx, first));
    }
}
