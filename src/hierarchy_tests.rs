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

fn pointer_event(position: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos: position,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

fn name_edit_id(project: &EditorProject, object: &Object) -> egui::Id {
    egui::Id::new((
        "property_object_name",
        project.ui_session_id().as_u128(),
        object.id().value(),
    ))
}

fn render_name_frame(
    ctx: &egui::Context,
    project: &mut EditorProject,
    original: &Object,
    draft: &mut Option<ObjectNameDraft>,
    events: Vec<egui::Event>,
) {
    let _ = ctx.run(input(events), |ctx| {
        egui::CentralPanel::default().show(ctx, |ui| {
            finish_object_name_on_switch(project, draft);
            render_object_name(ui, project, original, draft);
        });
    });
}

fn type_name(
    ctx: &egui::Context,
    project: &mut EditorProject,
    original: &Object,
    draft: &mut Option<ObjectNameDraft>,
    name: &str,
) {
    let id = name_edit_id(project, original);
    let original_name = name_of(project, original);
    ctx.memory_mut(|memory| memory.request_focus(id));
    render_name_frame(ctx, project, original, draft, Vec::new());
    for _ in original_name.chars() {
        render_name_frame(
            ctx,
            project,
            original,
            draft,
            vec![egui::Event::Key {
                key: egui::Key::Backspace,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            }],
        );
    }
    for character in name.chars() {
        render_name_frame(
            ctx,
            project,
            original,
            draft,
            vec![egui::Event::Text(character.to_string())],
        );
    }
}

fn name_test_project() -> (EditorProject, Object) {
    let mut object = ag_iso_terminal_designer::default_object(ObjectType::OutputRectangle);
    object.mut_id().set_value(2).unwrap();
    let mut pool = ObjectPool::default();
    pool.add(object.clone());
    let mut project = EditorProject::from(pool);
    project
        .get_mut_selected()
        .set(NullableObjectId::new(object.id().value()));
    project.update_selected();
    (project, object)
}

fn name_of(project: &EditorProject, object: &Object) -> String {
    project.get_object_info(object).get_name(object)
}

#[test]
fn property_name_is_drafted_per_letter_and_committed_once_on_enter_or_focus_loss() {
    for finish_with_enter in [true, false] {
        let (mut project, original) = name_test_project();
        let ctx = egui::Context::default();
        let mut draft = None;
        let original_name = name_of(&project, &original);
        let initial_revision = project.revision();

        type_name(&ctx, &mut project, &original, &mut draft, "Renamed object");
        assert_eq!(name_of(&project, &original), original_name);
        assert_eq!(project.revision(), initial_revision);

        if finish_with_enter {
            render_name_frame(
                &ctx,
                &mut project,
                &original,
                &mut draft,
                vec![egui::Event::Key {
                    key: egui::Key::Enter,
                    physical_key: None,
                    pressed: true,
                    repeat: false,
                    modifiers: egui::Modifiers::NONE,
                }],
            );
        } else {
            let outside = egui::pos2(380.0, 280.0);
            render_name_frame(
                &ctx,
                &mut project,
                &original,
                &mut draft,
                vec![
                    egui::Event::PointerMoved(outside),
                    pointer_event(outside, true),
                ],
            );
            render_name_frame(
                &ctx,
                &mut project,
                &original,
                &mut draft,
                vec![pointer_event(outside, false)],
            );
        }

        assert_eq!(name_of(&project, &original), "Renamed object");
        assert_eq!(project.revision(), initial_revision + 1);
        assert!(draft.is_none());
    }
}

#[test]
fn property_name_escape_discards_the_draft() {
    let (mut project, original) = name_test_project();
    let ctx = egui::Context::default();
    let mut draft = None;
    let original_name = name_of(&project, &original);
    let initial_revision = project.revision();

    type_name(&ctx, &mut project, &original, &mut draft, "Discard this");
    render_name_frame(
        &ctx,
        &mut project,
        &original,
        &mut draft,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );

    assert_eq!(name_of(&project, &original), original_name);
    assert_eq!(project.revision(), initial_revision);
    assert!(draft.is_none());
}

#[test]
fn property_name_selection_change_commits_the_draft_once() {
    let (mut project, original) = name_test_project();
    let ctx = egui::Context::default();
    let mut draft = None;
    let initial_revision = project.revision();

    type_name(
        &ctx,
        &mut project,
        &original,
        &mut draft,
        "Selection commit",
    );
    assert_eq!(project.revision(), initial_revision);

    project.get_mut_selected().set(NullableObjectId::default());
    project.update_selected();
    render_name_frame(&ctx, &mut project, &original, &mut draft, Vec::new());

    assert_eq!(name_of(&project, &original), "Selection commit");
    assert_eq!(project.revision(), initial_revision + 1);
    assert!(draft.is_none());
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
                pointer_event(*position, true),
            ]);
            frame(vec![pointer_event(*position, false)]);
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
        selection: None,
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
    let mut mask = ag_iso_terminal_designer::default_object(ObjectType::DataMask);
    mask.mut_id().set_value(1).unwrap();
    let mut pointer = ag_iso_terminal_designer::default_object(ObjectType::ObjectPointer);
    pointer.mut_id().set_value(2).unwrap();
    if let Object::ObjectPointer(pointer) = &mut pointer {
        pointer.value = NullableObjectId::new(3);
    }
    let mut target = ag_iso_terminal_designer::default_object(ObjectType::OutputRectangle);
    target.mut_id().set_value(3).unwrap();
    if let Object::DataMask(mask) = &mut mask {
        mask.object_refs.push(ag_iso_stack::object_pool::ObjectRef {
            id: ObjectId::new(2).unwrap(),
            offset: Point { x: 0, y: 0 },
        });
    }
    pool.add(mask);
    pool.add(pointer);
    pool.add(target);
    let mut project = EditorProject::from(pool);
    project.get_mut_selected().set(NullableObjectId::new(3));
    project.update_selected();
    let mask = project
        .get_pool()
        .object_by_id(ObjectId::new(1).unwrap())
        .unwrap();
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
                mask,
                &project,
                &mut masks,
                &mut canvas,
                &mut vec![0],
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
    let click = pointer_position + egui::vec2(5.0, 6.0);
    for events in [
        vec![egui::Event::PointerMoved(click), pointer_event(click, true)],
        vec![pointer_event(click, false)],
    ] {
        let _ = ctx.run(input(events), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                render_mask_object_hierarchy(
                    ui,
                    egui::Id::new("tree"),
                    1,
                    mask,
                    &project,
                    &mut masks,
                    &mut canvas,
                    &mut vec![0],
                    &mut HashSet::new(),
                    None,
                    true,
                    None,
                );
            });
        });
    }
    assert_eq!(project.get_mut_selected().get().0.unwrap().value(), 2);
}

#[test]
fn hierarchy_pointer_gestures_select_exact_rows_and_preserve_context_selection() {
    let mut pool = ObjectPool::default();
    let mut mask = ag_iso_terminal_designer::default_object(ObjectType::DataMask);
    mask.mut_id().set_value(1).unwrap();
    if let Object::DataMask(mask) = &mut mask {
        mask.object_refs = [2, 3, 2]
            .into_iter()
            .map(|id| ag_iso_stack::object_pool::ObjectRef {
                id: ObjectId::new(id).unwrap(),
                offset: Point { x: 10, y: 10 },
            })
            .collect();
    }
    pool.add(mask);
    for id in [2, 3] {
        let mut object = ag_iso_terminal_designer::default_object(ObjectType::OutputRectangle);
        object.mut_id().set_value(id).unwrap();
        pool.add(object);
    }
    let mut project = EditorProject::from(pool);
    project.get_mut_selected().set(NullableObjectId::new(2));
    let labels: Vec<_> = [2, 3]
        .into_iter()
        .map(|id| {
            project.object_display_name(
                project
                    .get_pool()
                    .object_by_id(ObjectId::new(id).unwrap())
                    .unwrap(),
            )
        })
        .collect();
    let ctx = egui::Context::default();
    let mut canvas = CanvasState::default();
    let mut masks = MaskViewState::default();
    masks.focus(1);
    let mut frame = |raw: egui::RawInput| {
        project.update_selected();
        ctx.run(raw, |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                begin_hierarchy_frame(ui, &project, &mut canvas);
                render_focused_mask_hierarchy(ui, &project, &mut masks, &mut canvas);
                finish_hierarchy_click(ui, &project, &mut canvas);
            });
        })
    };
    frame(input(Vec::new()));
    let mut settled = input(Vec::new());
    settled.time = Some(1.0);
    let output = frame(settled);
    let mut rows: Vec<_> = output
        .shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Text(text) if labels.iter().any(|label| label == text.galley.text()) => {
                Some(text.pos + egui::vec2(5.0, 6.0))
            }
            _ => None,
        })
        .collect();
    rows.sort_by(|a, b| a.y.total_cmp(&b.y));
    assert_eq!(
        rows.len(),
        3,
        "{:?}",
        output
            .shapes
            .iter()
            .filter_map(|shape| match &shape.shape {
                egui::Shape::Text(text) => Some(text.galley.text()),
                _ => None,
            })
            .collect::<Vec<_>>()
    );
    let mut click = |index: usize, modifiers: egui::Modifiers, button| {
        for pressed in [true, false] {
            let mut raw = input(vec![
                egui::Event::PointerMoved(rows[index]),
                egui::Event::PointerButton {
                    pos: rows[index],
                    button,
                    pressed,
                    modifiers,
                },
            ]);
            raw.modifiers = modifiers;
            frame(raw);
        }
    };
    click(0, egui::Modifiers::NONE, egui::PointerButton::Primary);
    click(
        2,
        egui::Modifiers {
            ctrl: true,
            ..Default::default()
        },
        egui::PointerButton::Primary,
    );
    // A context click on either of the repeated references keeps both selected.
    click(0, egui::Modifiers::NONE, egui::PointerButton::Secondary);
    drop(click);
    drop(frame);
    assert_eq!(
        canvas
            .selected_hierarchy_occurrences()
            .iter()
            .map(|row| row.path.clone())
            .collect::<Vec<_>>(),
        vec![vec![0, 0], vec![0, 2]]
    );
}
