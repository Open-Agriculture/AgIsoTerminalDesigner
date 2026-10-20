use super::*;

fn input(events: Vec<egui::Event>) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(400.0, 300.0),
        )),
        events,
        ..Default::default()
    }
}

fn pointer(position: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: position,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

/// Consecutive allocations used to reuse the previous row's interaction ID.
#[test]
fn clicking_rows_selects_the_painted_row_in_flat_expanded_and_scrolled_layouts() {
    for (expanded, scroll) in [(false, 0.0), (true, 0.0), (true, 45.0)] {
        let ctx = egui::Context::default();
        let mut rows = Vec::new();
        let mut clicked = Vec::new();
        let mut frame = |events| {
            rows.clear();
            let _ = ctx.run(input(events), |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .vertical_scroll_offset(scroll)
                        .show(ui, |ui| {
                            if expanded {
                                ui_style::collapsing_section(
                                    ui,
                                    egui::Id::new("branch"),
                                    true,
                                    |ui| {
                                        tree_row_with_id(
                                            ui,
                                            egui::Id::new("parent"),
                                            false,
                                            "Parent",
                                        );
                                    },
                                    |ui| {
                                        for index in 0..10 {
                                            // Same label deliberately represents repeated object occurrences.
                                            let row = tree_row_with_id(
                                                ui,
                                                egui::Id::new(("row", index)),
                                                false,
                                                "Repeated object",
                                            );
                                            rows.push(row.rect);
                                            if row.clicked() {
                                                clicked.push(index);
                                            }
                                        }
                                    },
                                );
                            } else {
                                for index in 0..10 {
                                    let row = tree_row_with_id(
                                        ui,
                                        egui::Id::new(("row", index)),
                                        false,
                                        "Repeated object",
                                    );
                                    rows.push(row.rect);
                                    if row.clicked() {
                                        clicked.push(index);
                                    }
                                }
                            }
                        });
                });
            });
            rows.clone()
        };
        frame(Vec::new());
        let geometry = frame(Vec::new());
        let visible: Vec<_> = geometry
            .iter()
            .enumerate()
            .filter(|(_, rect)| rect.top() > 40.0 && rect.bottom() < 260.0)
            .map(|(index, rect)| (index, rect.center()))
            .collect();
        for (_, position) in &visible {
            frame(vec![
                egui::Event::PointerMoved(*position),
                pointer(*position, true),
            ]);
            frame(vec![pointer(*position, false)]);
        }
        assert_eq!(
            clicked,
            visible.iter().map(|(index, _)| *index).collect::<Vec<_>>(),
            "expanded={expanded}, scroll={scroll}"
        );
    }
}

#[test]
fn drag_zones_choose_sibling_slots_or_parent_without_consuming_payload() {
    let ctx = egui::Context::default();
    let source = TreeDragPayload {
        root: 1,
        source_parent_path: vec![0],
        sibling_index: 2,
        parent_id: 1,
        occurrence_id: egui::Id::new("source"),
        label: "Source".into(),
        target: ag_iso_terminal_designer::canvas::edits::PlacementTarget::Primitive {
            parent_id: 1,
            child_index: 2,
        },
    };
    let draw = |ctx: &egui::Context| {
        let mut rect = egui::Rect::NOTHING;
        egui::CentralPanel::default().show(ctx, |ui| {
            let row = tree_row_with_id(ui, egui::Id::new("destination"), false, "Destination");
            rect = row.rect;
            hierarchy_drag_row(ui, &row, Some(&source), Some((1, 7, vec![0, 3])));
        });
        rect
    };
    let mut rect = egui::Rect::NOTHING;
    let _ = ctx.run(input(Vec::new()), |ctx| {
        rect = draw(ctx);
    });
    for (y, expected_slot, expected_parent) in [
        (rect.top() + 1.0, Some(2), 1),
        (rect.center().y, None, 7),
        (rect.bottom() - 1.0, Some(3), 1),
    ] {
        egui::DragAndDrop::set_payload(&ctx, source.clone());
        let _ = ctx.run(
            input(vec![egui::Event::PointerMoved(egui::pos2(
                rect.center().x,
                y,
            ))]),
            |ctx| {
                draw(ctx);
            },
        );
        let candidate = ctx
            .data(|data| data.get_temp::<TreeDropCandidate>(egui::Id::new("tree_drop_candidate")))
            .unwrap();
        assert_eq!(
            (candidate.insertion_index, candidate.parent_id),
            (expected_slot, expected_parent)
        );
        assert!(egui::DragAndDrop::has_payload_of_type::<TreeDragPayload>(
            &ctx
        ));
    }
}

#[test]
fn pointer_target_is_the_next_visible_row_without_an_extra_heading() {
    let mut pool = ObjectPool::default();
    let mut pointer = ag_iso_terminal_designer::default_object(ObjectType::ObjectPointer);
    pointer.mut_id().set_value(2).unwrap();
    if let Object::ObjectPointer(pointer) = &mut pointer {
        pointer.value = NullableObjectId::new(3);
    }
    let mut target = ag_iso_terminal_designer::default_object(ObjectType::OutputRectangle);
    target.mut_id().set_value(3).unwrap();
    pool.add(pointer);
    pool.add(target);
    let mut project = EditorProject::from(pool);
    project.get_mut_selected().set(NullableObjectId::new(3));
    project.update_selected();
    let pointer = project
        .get_pool()
        .object_by_id(ObjectId::new(2).unwrap())
        .unwrap();
    let target = project
        .get_pool()
        .object_by_id(ObjectId::new(3).unwrap())
        .unwrap();
    let pointer_label = project.object_display_name(pointer);
    let target_label = project.object_display_name(target);
    let ctx = egui::Context::default();
    let mut canvas = CanvasState::default();
    let mut masks = MaskViewState::default();
    let output = ctx.run(input(Vec::new()), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            render_mask_object_hierarchy(
                ui,
                egui::Id::new("tree"),
                1,
                pointer,
                &project,
                &mut masks,
                &mut canvas,
                &mut vec![0, 0],
                &mut HashSet::new(),
                None,
                true,
                None,
            );
        });
    });
    let text: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) => Some((text.galley.text(), text.pos)),
            _ => None,
        })
        .collect();
    assert!(!text
        .iter()
        .any(|(label, _)| *label == "Active pointer target"));
    let pointer_position = text
        .iter()
        .find(|(label, _)| *label == pointer_label)
        .unwrap()
        .1;
    let target_position = text
        .iter()
        .find(|(label, _)| *label == target_label)
        .unwrap()
        .1;
    assert!(target_position.y > pointer_position.y);
    assert!(
        target_position.y - pointer_position.y < 40.0,
        "target should immediately follow pointer"
    );
}
