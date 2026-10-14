use super::*;
use crate::{default_object, ObjectTarget, Operation, OperationTransaction};
use ag_iso_stack::object_pool::{object::Object, ObjectPool, ObjectRef, ObjectType};

fn fixture(nested: bool) -> EditorProject {
    let mut pool = ObjectPool::default();
    let mut mask = default_object(ObjectType::DataMask);
    mask.mut_id().set_value(1).unwrap();
    let mut shape = default_object(ObjectType::OutputRectangle);
    shape.mut_id().set_value(2).unwrap();
    if let Object::OutputRectangle(shape) = &mut shape {
        shape.width = 40;
        shape.height = 30;
    }
    let reference = |id, x, y| ObjectRef {
        id: ObjectId::new(id).unwrap(),
        offset: Point { x, y },
    };
    if nested {
        let mut container = default_object(ObjectType::Container);
        container.mut_id().set_value(3).unwrap();
        if let Object::Container(c) = &mut container {
            c.width = 150;
            c.height = 150;
            c.object_refs = vec![reference(2, 20, 20)];
        }
        if let Object::DataMask(m) = &mut mask {
            m.object_refs = vec![reference(3, 100, 100)];
        }
        pool.add(container);
    } else if let Object::DataMask(m) = &mut mask {
        m.object_refs = vec![reference(2, 100, 100), reference(2, 220, 100)];
    }
    pool.add(mask);
    pool.add(shape);
    let mut project = EditorProject::from(pool);
    project.mask_size = 500;
    project
}

struct Harness {
    ctx: egui::Context,
    state: CanvasState,
    project: EditorProject,
    time: f64,
    output: egui::FullOutput,
    inspector: Option<(Rect, Rect)>,
    masks: Option<super::super::MaskViewState>,
}
impl Harness {
    fn new(nested: bool) -> Self {
        let mut h = Self {
            ctx: egui::Context::default(),
            state: CanvasState::default(),
            project: fixture(nested),
            time: 0.0,
            output: egui::FullOutput::default(),
            inspector: None,
            masks: None,
        };
        h.frame(vec![], Modifiers::NONE);
        h
    }
    fn frame(&mut self, events: Vec<egui::Event>, modifiers: Modifiers) {
        self.time += 0.05;
        let materialized = self.project.materialize().unwrap();
        self.output = self.ctx.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, egui::vec2(1000.0, 800.0))),
                time: Some(self.time),
                events,
                modifiers,
                ..Default::default()
            },
            |ctx| {
                if let Some(inspector) = &mut self.inspector {
                    let panel = egui::SidePanel::right("right_panel").show(ctx, |ui| {
                        inspector.1 = ui.button("Configuration control").rect;
                        ui.allocate_space(ui.available_size());
                    });
                    inspector.0 = panel.response.rect;
                }
                egui::CentralPanel::default().show(ctx, |ui| {
                    if let Some(masks) = &mut self.masks {
                        self.state
                            .show_masks(ui, &mut self.project, &materialized, masks);
                    } else {
                        self.state.show(ui, &mut self.project, &materialized, 1);
                    }
                });
            },
        );
        self.project.update_selected();
    }
    fn screen(&self, p: Pos2) -> Pos2 {
        self.state.viewport.min + self.state.pan + p.to_vec2() * self.state.zoom
    }
    fn pointer(&mut self, p: Pos2, pressed: Option<bool>, mods: Modifiers) {
        let mut events = vec![egui::Event::PointerMoved(p)];
        if let Some(pressed) = pressed {
            events.push(egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: mods,
            });
        }
        self.frame(events, mods);
    }
    fn key(&mut self, key: Key, mods: Modifiers) {
        self.frame(
            vec![egui::Event::Key {
                key,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: mods,
            }],
            mods,
        );
    }
    fn offset(&self, index: usize) -> (i16, i16) {
        position(
            &self.project,
            &PlacementTarget::Primitive {
                parent_id: 1,
                child_index: index,
            },
        )
        .unwrap()
    }
}

#[test]
fn inspector_clicks_preserve_selection_and_empty_canvas_click_clears_it() {
    let mut h = Harness::new(false);
    h.inspector = Some((Rect::NOTHING, Rect::NOTHING));
    h.frame(vec![], Modifiers::NONE);
    h.frame(vec![], Modifiers::NONE);
    let object = h.screen(egui::pos2(110.0, 110.0));
    h.pointer(object, None, Modifiers::NONE);
    h.pointer(object, Some(true), Modifiers::NONE);
    h.pointer(object, Some(false), Modifiers::NONE);
    assert_eq!(h.project.get_selected(), NullableObjectId::new(2));
    assert_eq!(h.state.selected, vec![vec![0]]);

    let (panel, control) = h.inspector.unwrap();
    for p in [panel.center(), control.center()] {
        h.pointer(p, None, Modifiers::NONE);
        h.pointer(p, Some(true), Modifiers::NONE);
        h.pointer(p, Some(false), Modifiers::NONE);
        assert_eq!(h.project.get_selected(), NullableObjectId::new(2));
        assert_eq!(h.state.selected, vec![vec![0]]);
        assert!(h.state.gesture.is_none());
    }

    let empty = h.state.viewport.right_bottom() - egui::vec2(20.0, 20.0);
    h.pointer(empty, None, Modifiers::NONE);
    h.pointer(empty, Some(true), Modifiers::NONE);
    h.pointer(empty, Some(false), Modifiers::NONE);
    assert_eq!(h.project.get_selected(), NullableObjectId::NULL);
    assert!(h.state.selected.is_empty());
    assert!(!h.project.undo_available());
}

#[test]
fn drag_second_occurrence_at_zoom_commits_once_and_undoes() {
    let mut h = Harness::new(false);
    h.state.zoom = 2.0;
    h.frame(vec![], Modifiers::NONE);
    let p = h.screen(egui::pos2(235.0, 115.0));
    h.pointer(p, None, Modifiers::CTRL);
    h.pointer(p, Some(true), Modifiers::CTRL);
    assert_eq!(h.state.selected, vec![vec![1]]);
    h.pointer(p + egui::vec2(40.0, 18.0), None, Modifiers::CTRL);
    assert_eq!(h.offset(1), (220, 100));
    assert!(!h.project.undo_available());
    h.pointer(p + egui::vec2(60.0, 24.0), None, Modifiers::CTRL);
    h.pointer(p + egui::vec2(60.0, 24.0), Some(false), Modifiers::CTRL);
    assert_eq!(h.offset(0), (100, 100));
    assert_eq!(h.offset(1), (250, 112));
    assert_eq!(h.state.selected, vec![vec![1]]);
    assert!(h.project.undo_operation());
    assert_eq!(h.offset(1), (220, 100));
    assert!(!h.project.undo_available());
    assert!(h.project.redo_operation());
    assert_eq!(h.offset(1), (250, 112));
}

#[test]
fn escape_cancels_draft_and_noop_click_has_no_history() {
    let mut h = Harness::new(false);
    let p = h.screen(egui::pos2(110.0, 110.0));
    h.pointer(p, None, Modifiers::NONE);
    h.pointer(p, Some(true), Modifiers::NONE);
    h.pointer(p + egui::vec2(30.0, 20.0), None, Modifiers::NONE);
    assert!(h.state.gesture.as_ref().unwrap().draft.is_some());
    h.key(Key::Escape, Modifiers::NONE);
    h.pointer(p + egui::vec2(30.0, 20.0), Some(false), Modifiers::NONE);
    assert_eq!(h.offset(0), (100, 100));
    assert!(!h.project.undo_available());
    let p = h.screen(egui::pos2(110.0, 110.0));
    h.pointer(p, None, Modifiers::NONE);
    h.pointer(p, Some(true), Modifiers::NONE);
    h.pointer(p, Some(false), Modifiers::NONE);
    assert!(!h.project.undo_available());
}

#[test]
fn alt_shift_drag_copies_independently_and_constrains_axis() {
    let mut h = Harness::new(false);
    let mods = Modifiers {
        alt: true,
        shift: true,
        ctrl: true,
        ..Default::default()
    };
    let p = h.screen(egui::pos2(110.0, 110.0));
    h.pointer(p, None, mods);
    h.pointer(p, Some(true), mods);
    h.pointer(p + egui::vec2(50.0, 15.0), None, mods);
    h.pointer(p + egui::vec2(50.0, 15.0), Some(false), mods);
    let refs = crate::operations::object_refs(
        h.project
            .get_pool()
            .object_by_id(ObjectId::new(1).unwrap())
            .unwrap(),
    )
    .unwrap();
    assert_eq!(refs.len(), 3);
    assert_ne!(refs[2].id, refs[0].id);
    assert_eq!(h.offset(2), (150, 100));
    assert_eq!(h.offset(0), (100, 100));
    assert!(h.project.undo_operation());
    assert_eq!(h.project.get_pool().objects().len(), 2);
}

#[test]
fn double_click_enters_container_and_next_click_selects_child() {
    let mut h = Harness::new(true);
    let p = h.screen(egui::pos2(130.0, 130.0));
    h.pointer(p, None, Modifiers::NONE);
    for _ in 0..2 {
        h.pointer(p, Some(true), Modifiers::NONE);
        h.pointer(p, Some(false), Modifiers::NONE);
    }
    assert_eq!(h.state.context, vec![0]);
    assert_eq!(h.state.selected, vec![vec![0, 0]]);
    h.pointer(p, Some(true), Modifiers::NONE);
    h.pointer(p, Some(false), Modifiers::NONE);
    assert_eq!(h.state.selected, vec![vec![0, 0]]);
}

#[test]
fn external_structure_change_does_not_retarget_canvas_selection() {
    let mut h = Harness::new(false);
    let p = h.screen(egui::pos2(230.0, 110.0));
    h.pointer(p, None, Modifiers::NONE);
    h.pointer(p, Some(true), Modifiers::NONE);
    h.pointer(p, Some(false), Modifiers::NONE);
    let mut transaction = OperationTransaction::new(Some("Remove earlier placement".into()));
    transaction.operations.push(Operation::SetChildren {
        parent: ObjectTarget::id(1),
        children: vec![crate::ChildPlacement {
            child: ObjectTarget::id(2),
            x: 220,
            y: 100,
        }],
    });
    h.project.execute_transaction(transaction).unwrap();
    h.frame(vec![], Modifiers::NONE);
    assert_eq!(h.state.selected, vec![vec![0]]);
    h.key(Key::ArrowRight, Modifiers::NONE);
    assert_eq!(h.offset(0), (221, 100));
}

#[test]
fn geometry_snap_is_zoom_independent_and_ctrl_bypass_keeps_integer_pixels() {
    let mut project = fixture(false);
    project.mask_size = 600;
    let m = project.materialize().unwrap();
    let scene = geometry::build(&project, &m, 1);
    let item = scene
        .items
        .iter()
        .find(|i| i.path == vec![0])
        .unwrap()
        .clone();
    for zoom in [0.5, 1.0, 2.0] {
        let delta = egui::vec2(120.0 - 4.0 / zoom, 0.0);
        let (snapped, guides) =
            snap_delta(&[item.clone()], &scene, delta, zoom, true, false, false);
        assert_eq!(
            snapped.x, 120.0,
            "zoom {zoom}, delta {delta:?}, scene {:?}",
            scene.items
        );
        assert!(!guides.is_empty());
        let (free, guides) = snap_delta(&[item.clone()], &scene, delta, zoom, false, false, false);
        assert_eq!(free, delta.round());
        assert!(guides.is_empty());
    }
}
#[test]
fn numeric_dimension_drag_is_one_edit_and_updates_shared_object() {
    let mut h = Harness::new(false);
    let p = h.screen(egui::pos2(110.0, 110.0));
    h.pointer(p, None, Modifiers::NONE);
    h.pointer(p, Some(true), Modifiers::NONE);
    h.pointer(p, Some(false), Modifiers::NONE);
    h.frame(vec![], Modifiers::NONE);
    let field = h
        .output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text)
                if text.galley.job.text == "40" && text.pos.y < h.state.viewport.top() =>
            {
                Some(text.pos + text.galley.rect.center().to_vec2())
            }
            _ => None,
        })
        .expect("width field rendered");
    h.pointer(field, None, Modifiers::NONE);
    h.pointer(field, Some(true), Modifiers::NONE);
    h.pointer(field + egui::vec2(20.0, 0.0), None, Modifiers::NONE);
    let width = |project: &EditorProject| {
        crate::object_properties::get_object_property(
            project
                .get_pool()
                .object_by_id(ObjectId::new(2).unwrap())
                .unwrap(),
            "width",
        )
        .unwrap()
        .as_u64()
        .unwrap()
    };
    assert_eq!(width(&h.project), 40);
    assert!(!h.project.undo_available());
    h.pointer(field + egui::vec2(30.0, 0.0), Some(false), Modifiers::NONE);
    assert!(width(&h.project) > 40);
    let m = h.project.materialize().unwrap();
    let scene = geometry::build(&h.project, &m, 1);
    assert_eq!(scene.items[1].rect.width(), scene.items[2].rect.width());
    assert!(h.project.undo_operation());
    assert_eq!(width(&h.project), 40);
    assert!(!h.project.undo_available());
}
#[test]
fn resize_from_center_preserves_ratio_and_undoes_as_one_action() {
    let mut h = Harness::new(false);
    let p = h.screen(egui::pos2(110.0, 110.0));
    h.pointer(p, None, Modifiers::NONE);
    h.pointer(p, Some(true), Modifiers::NONE);
    h.pointer(p, Some(false), Modifiers::NONE);
    h.frame(vec![], Modifiers::NONE);
    let corner = h.screen(egui::pos2(140.0, 130.0));
    let mods = Modifiers {
        alt: true,
        shift: true,
        ctrl: true,
        ..Default::default()
    };
    h.pointer(corner, None, mods);
    h.pointer(corner, Some(true), mods);
    h.pointer(corner + egui::vec2(20.0, 15.0), None, mods);
    assert_eq!(h.offset(0), (100, 100));
    h.pointer(corner + egui::vec2(20.0, 15.0), Some(false), mods);
    assert_eq!(h.offset(0), (80, 85));
    let object = h
        .project
        .get_pool()
        .object_by_id(ObjectId::new(2).unwrap())
        .unwrap();
    assert_eq!(
        crate::object_properties::get_object_property(object, "width").unwrap(),
        serde_json::json!(80)
    );
    assert_eq!(
        crate::object_properties::get_object_property(object, "height").unwrap(),
        serde_json::json!(60)
    );
    assert_eq!(h.offset(1), (220, 100));
    assert!(h.project.undo_operation());
    assert_eq!(h.offset(0), (100, 100));
    assert!(!h.project.undo_available());
}
#[test]
fn releasing_alt_before_drop_moves_original_without_leaking_a_copy() {
    let mut h = Harness::new(false);
    let p = h.screen(egui::pos2(110.0, 110.0));
    let alt = Modifiers {
        alt: true,
        ctrl: true,
        ..Default::default()
    };
    h.pointer(p, None, alt);
    h.pointer(p, Some(true), alt);
    h.pointer(p + egui::vec2(50.0, 20.0), None, alt);
    assert_eq!(h.project.get_pool().objects().len(), 2);
    h.pointer(p + egui::vec2(50.0, 20.0), Some(false), Modifiers::CTRL);
    assert_eq!(h.project.get_pool().objects().len(), 2);
    assert_eq!(h.offset(0), (150, 120));
    assert_eq!(h.offset(1), (220, 100));
}
#[test]
fn drag_is_clamped_to_mask_and_source_changes_cancel_drafts() {
    let mut h = Harness::new(false);
    h.state.snap_objects = false;
    h.state.snap_spacing = false;
    let p = h.screen(egui::pos2(110.0, 110.0));
    h.pointer(p, None, Modifiers::CTRL);
    h.pointer(p, Some(true), Modifiers::CTRL);
    h.pointer(p - egui::vec2(130.0, 120.0), None, Modifiers::CTRL);
    h.pointer(p - egui::vec2(130.0, 120.0), Some(false), Modifiers::CTRL);
    assert_eq!(h.offset(0), (0, 0));
    assert!(h.project.undo_operation());
    h.frame(vec![], Modifiers::NONE);
    let p = h.screen(egui::pos2(110.0, 110.0));
    h.pointer(p, None, Modifiers::CTRL);
    h.pointer(p, Some(true), Modifiers::CTRL);
    h.pointer(p + egui::vec2(40.0, 20.0), None, Modifiers::CTRL);
    h.project.set_mask_size(600).unwrap();
    h.frame(vec![], Modifiers::CTRL);
    assert!(h.state.gesture.is_none());
    h.pointer(p + egui::vec2(40.0, 20.0), Some(false), Modifiers::CTRL);
    assert_eq!(h.offset(0), (100, 100));
}

#[test]
fn moving_a_group_clamps_one_shared_delta_for_all_members() {
    let project = fixture(false);
    let materialized = project.materialize().unwrap();
    let scene = geometry::build(&project, &materialized, 1);
    let items = scene.items[1..].to_vec();
    let edits = move_edits(&project, &items, egui::vec2(400.0, 0.0), false);
    let positions: Vec<_> = edits
        .iter()
        .filter_map(|edit| match edit {
            CanvasEdit::Move { x, y, .. } => Some((*x, *y)),
            _ => None,
        })
        .collect();
    assert_eq!(positions, vec![(340, 100), (460, 100)]);
}

#[test]
fn movement_clamp_preserves_tangential_motion_at_a_parent_edge() {
    let mut project = fixture(false);
    edits::apply(
        &mut project,
        "Move to right edge",
        &[CanvasEdit::Move {
            target: PlacementTarget::Primitive {
                parent_id: 1,
                child_index: 0,
            },
            x: 460,
            y: 100,
        }],
    )
    .unwrap();
    let materialized = project.materialize().unwrap();
    let scene = geometry::build(&project, &materialized, 1);
    let item = scene
        .items
        .iter()
        .find(|item| item.path == vec![0])
        .unwrap();
    let edits = move_edits(&project, &[item.clone()], egui::vec2(10.0, 20.0), false);
    assert!(matches!(
        edits.as_slice(),
        [CanvasEdit::Move { x: 460, y: 120, .. }]
    ));
}

#[test]
fn duplicate_clamps_to_the_parent_edge_without_moving_the_source() {
    let project = fixture(false);
    let materialized = project.materialize().unwrap();
    let scene = geometry::build(&project, &materialized, 1);
    let item = scene
        .items
        .iter()
        .find(|item| item.path == vec![1])
        .unwrap();
    let edits = move_edits(&project, &[item.clone()], egui::vec2(400.0, 0.0), true);
    assert!(matches!(
        edits.as_slice(),
        [CanvasEdit::Duplicate { x: 460, y: 100, .. }]
    ));
}

#[test]
fn shared_resize_clamps_to_the_tightest_placement() {
    let project = fixture(false);
    let materialized = project.materialize().unwrap();
    let scene = geometry::build(&project, &materialized, 1);
    let item = &scene.items[1];
    let edits = resize_edits(
        &project,
        item,
        egui::vec2(1.0, 0.0),
        egui::vec2(500.0, 0.0),
        Modifiers::NONE,
    );
    let width = edits.iter().find_map(|edit| match edit {
        CanvasEdit::Resize { width, .. } => *width,
        _ => None,
    });
    assert_eq!(width, Some(280));
}

#[test]
fn editing_x_does_not_change_existing_zero_dimensions() {
    let mut h = Harness::new(false);
    let mut transaction = OperationTransaction::new(Some("Zero-sized object".into()));
    transaction.operations.push(Operation::SetProperty {
        object: ObjectTarget::id(2),
        property: "width".into(),
        value: serde_json::json!(0),
    });
    h.project.execute_transaction(transaction).unwrap();
    // Selecting from the hierarchy also permits editing zero-size objects.
    h.project.get_mut_selected().set(NullableObjectId::new(2));
    h.project.update_selected();
    h.frame(vec![], Modifiers::NONE);
    let draft = h.state.field_draft.as_ref().unwrap();
    assert_eq!(draft.width, Some(0));
    assert!(!draft.changed);
    let field = h
        .output
        .shapes
        .iter()
        .find_map(|shape| match &shape.shape {
            egui::Shape::Text(text)
                if text.galley.job.text == "100" && text.pos.y < h.state.viewport.top() =>
            {
                Some(text.pos + text.galley.rect.center().to_vec2())
            }
            _ => None,
        })
        .expect("X field rendered");
    h.pointer(field, None, Modifiers::NONE);
    h.pointer(field, Some(true), Modifiers::NONE);
    h.pointer(field + egui::vec2(10.0, 0.0), None, Modifiers::NONE);
    h.pointer(field + egui::vec2(20.0, 0.0), Some(false), Modifiers::NONE);
    assert!(h.offset(0).0 > 100);
    let object = h
        .project
        .get_pool()
        .object_by_id(ObjectId::new(2).unwrap())
        .unwrap();
    assert_eq!(
        crate::object_properties::get_object_property(object, "width").unwrap(),
        serde_json::json!(0)
    );
    assert_eq!(
        crate::object_properties::get_object_property(object, "height").unwrap(),
        serde_json::json!(30)
    );
}

#[test]
fn resize_keeps_zero_dimensions_when_no_larger_size_fits() {
    let mut project = fixture(false);
    let mut transaction = OperationTransaction::new(Some("Zero viewport".into()));
    transaction.operations.push(Operation::SetProperty {
        object: ObjectTarget::id(2),
        property: "width".into(),
        value: serde_json::json!(0),
    });
    transaction.operations.push(Operation::SetProperty {
        object: ObjectTarget::id(2),
        property: "height".into(),
        value: serde_json::json!(0),
    });
    transaction.operations.push(Operation::SetChildren {
        parent: ObjectTarget::id(1),
        children: vec![crate::ChildPlacement {
            child: ObjectTarget::id(2),
            x: 0,
            y: 0,
        }],
    });
    project.execute_transaction(transaction).unwrap();
    // Legacy projects may retain a zero viewport; new settings require positive dimensions.
    project.mask_size = 0;
    let materialized = project.materialize().unwrap();
    let scene = geometry::build(&project, &materialized, 1);
    let item = scene.items.iter().find(|i| i.target.is_some()).unwrap();
    let edits = resize_edits(
        &project,
        item,
        Vec2::splat(1.0),
        Vec2::splat(10.0),
        Modifiers::NONE,
    );
    let revision = project.revision();
    edits::apply(&mut project, "Resize at boundary", &edits).unwrap();
    assert_eq!(project.revision(), revision);
    assert!(
        matches!(project.get_pool().object_by_id(ObjectId::new(2).unwrap()),
        Some(Object::OutputRectangle(rect)) if rect.width == 0 && rect.height == 0)
    );
}

#[test]
fn equal_spacing_guides_match_the_snapped_gaps_before_and_after_neighbors() {
    let mut project = fixture(false);
    let mut transaction = OperationTransaction::new(Some("Third placement".into()));
    transaction.operations.push(Operation::AddChild {
        parent: ObjectTarget::id(1),
        child: ObjectTarget::id(2),
        x: 400,
        y: 100,
    });
    project.execute_transaction(transaction).unwrap();
    let m = project.materialize().unwrap();
    let scene = geometry::build(&project, &m, 1);
    let item = scene
        .items
        .iter()
        .find(|i| i.path == vec![2])
        .unwrap()
        .clone();
    for (delta, expected) in [
        (egui::vec2(-63.0, 0.0), -60.0),
        (egui::vec2(-423.0, 0.0), -420.0),
    ] {
        let (snapped, guides) = snap_delta(&[item.clone()], &scene, delta, 1.0, false, true, false);
        assert_eq!(snapped.x, expected);
        assert_eq!(guides.len(), 2);
        assert_eq!(guides[0].1.x - guides[0].0.x, 80.0);
        assert_eq!(guides[1].1.x - guides[1].0.x, 80.0);
    }
}

#[test]
fn rendered_vt_button_does_not_capture_canvas_drag() {
    let mut h = Harness::new(false);
    h.project
        .edit_components("Button fixture", |pool, _| {
            let mut button = default_object(ObjectType::Button);
            button.mut_id().set_value(2).unwrap();
            if let Object::Button(b) = &mut button {
                b.width = 40;
                b.height = 30;
            }
            *pool.object_mut_by_id(ObjectId::new(2).unwrap()).unwrap() = button;
            Ok(())
        })
        .unwrap();
    h.frame(vec![], Modifiers::NONE);
    let p = h.screen(egui::pos2(110.0, 110.0));
    h.pointer(p, None, Modifiers::CTRL);
    h.pointer(p, Some(true), Modifiers::CTRL);
    h.pointer(p + egui::vec2(20.0, 10.0), None, Modifiers::CTRL);
    h.pointer(p + egui::vec2(20.0, 10.0), Some(false), Modifiers::CTRL);
    assert_eq!(h.offset(0), (120, 110));
    assert_eq!(h.offset(1), (220, 100));
    assert_eq!(h.state.selected, vec![vec![0]]);
}

#[test]
fn softkey_contents_edit_in_local_coordinates_and_page_changes_clear_selection() {
    let mut h = Harness::new(false);
    let mut pool = h.project.get_pool().clone();
    if let Some(Object::DataMask(mask)) = pool.object_mut_by_id(ObjectId::new(1).unwrap()) {
        mask.soft_key_mask = NullableObjectId::new(10);
    }
    let mut softmask = default_object(ObjectType::SoftKeyMask);
    softmask.mut_id().set_value(10).unwrap();
    if let Object::SoftKeyMask(mask) = &mut softmask {
        mask.objects = vec![ObjectId::new(11).unwrap(); 13];
    }
    let mut key = default_object(ObjectType::Key);
    key.mut_id().set_value(11).unwrap();
    if let Object::Key(key) = &mut key {
        key.background_colour = 2;
        key.object_refs.push(ObjectRef {
            id: ObjectId::new(2).unwrap(),
            offset: Point { x: 5, y: 5 },
        });
    }
    pool.add(softmask);
    pool.add(key);
    h.project = EditorProject::from(pool);
    h.project.mask_size = 500;
    h.state = CanvasState::default();
    h.frame(vec![], Modifiers::NONE);
    let keys = super::super::softkeys::build(&h.project, h.project.get_pool(), 1, 0);
    let rect = keys.slots[0].rect;
    let screen_rect = Rect::from_min_size(h.screen(rect.min), rect.size() * h.state.zoom);
    let key_fill = crate::effective_colour::colour(h.project.get_pool(), 2).convert();
    assert!(h.output.shapes.iter().any(|shape| matches!(&shape.shape,
        egui::Shape::Rect(painted) if painted.rect == screen_rect && painted.fill == key_fill
            && shape.clip_rect == screen_rect)));
    let background = h.screen(rect.min + egui::vec2(55.0, 55.0));
    h.pointer(background, Some(true), Modifiers::NONE);
    h.pointer(background, Some(false), Modifiers::NONE);
    assert_eq!(h.project.get_selected(), NullableObjectId::new(11));
    let child = h.screen(rect.min + egui::vec2(15.0, 15.0));
    h.pointer(child, Some(true), Modifiers::NONE);
    h.pointer(child + egui::vec2(5.0, 0.0), None, Modifiers::CTRL);
    h.pointer(child + egui::vec2(5.0, 0.0), Some(false), Modifiers::CTRL);
    assert_eq!(
        position(
            &h.project,
            &PlacementTarget::Primitive {
                parent_id: 11,
                child_index: 0
            }
        ),
        Some((10, 5))
    );
    assert_eq!(h.project.get_selected(), NullableObjectId::new(2));
    assert_eq!(h.state.selected.len(), 1);
    let next = h.screen(keys.slots[11].rect.center());
    h.pointer(next, Some(true), Modifiers::NONE);
    h.pointer(next, Some(false), Modifiers::NONE);
    assert_eq!(h.state.softkey_page, 1);
    assert!(h.state.selected.is_empty());
}

#[test]
fn multiple_mask_pairs_keep_edits_local_and_softkey_pages_independent() {
    let mut h = Harness::new(false);
    let mut pool = h.project.get_pool().clone();
    if let Some(Object::DataMask(mask)) = pool.object_mut_by_id(ObjectId::new(1).unwrap()) {
        mask.soft_key_mask = NullableObjectId::new(10);
    }
    let mut alarm = default_object(ObjectType::AlarmMask);
    alarm.mut_id().set_value(20).unwrap();
    if let Object::AlarmMask(mask) = &mut alarm {
        mask.soft_key_mask = NullableObjectId::new(10);
    }
    let mut softmask = default_object(ObjectType::SoftKeyMask);
    softmask.mut_id().set_value(10).unwrap();
    if let Object::SoftKeyMask(mask) = &mut softmask {
        mask.objects = vec![ObjectId::new(11).unwrap(); 13];
    }
    let mut key = default_object(ObjectType::Key);
    key.mut_id().set_value(11).unwrap();
    if let Object::Key(key) = &mut key {
        key.background_colour = 2;
        key.object_refs.push(ObjectRef {
            id: ObjectId::new(2).unwrap(),
            offset: Point { x: 5, y: 5 },
        });
    }
    pool.add(alarm);
    pool.add(softmask);
    pool.add(key);
    h.project = EditorProject::from(pool);
    h.project.mask_size = 500;
    h.state = CanvasState::default();
    let mut masks = super::super::MaskViewState::default();
    masks.reconcile(&h.project);
    masks.set_visible(20, true);
    h.masks = Some(masks);
    h.frame(vec![], Modifiers::NONE);
    let keys = super::super::softkeys::build(&h.project, h.project.get_pool(), 1, 0);
    let pair = Rect::from_min_size(Pos2::ZERO, egui::vec2(500.0, 500.0)).union(keys.rect);
    let second = egui::vec2(pair.width() + 32.0, 24.0);
    let slot = keys.slots[0].rect;
    let child = h.screen(slot.min + second + egui::vec2(15.0, 15.0));
    h.pointer(child, Some(true), Modifiers::CTRL);
    h.pointer(
        child + egui::vec2(5.0 * h.state.zoom, 0.0),
        None,
        Modifiers::CTRL,
    );
    h.pointer(
        child + egui::vec2(5.0 * h.state.zoom, 0.0),
        Some(false),
        Modifiers::CTRL,
    );
    assert_eq!(h.masks.as_ref().unwrap().focused, Some(20));
    assert_eq!(
        position(
            &h.project,
            &PlacementTarget::Primitive {
                parent_id: 11,
                child_index: 0,
            }
        ),
        Some((10, 5))
    );
    assert_eq!(h.project.get_selected(), NullableObjectId::new(2));

    // A later dock pane can change focus before a numeric draft is completed.
    h.frame(vec![], Modifiers::NONE);
    let draft = h.state.field_draft.as_mut().unwrap();
    draft.position = Some((30, 5));
    draft.changed = true;
    draft.finish_pending = true;
    h.masks.as_mut().unwrap().focus(1);
    h.frame(vec![], Modifiers::NONE);
    assert_eq!(
        position(
            &h.project,
            &PlacementTarget::Primitive {
                parent_id: 11,
                child_index: 0,
            }
        ),
        Some((10, 5))
    );
    h.masks.as_mut().unwrap().focus(20);
    h.frame(vec![], Modifiers::NONE);

    // Both owners render the shared key, but changing one owner's page is local.
    let key_fill = crate::effective_colour::colour(h.project.get_pool(), 2).convert();
    for offset in [egui::vec2(0.0, 24.0), second] {
        let rect = Rect::from_min_size(h.screen(slot.min + offset), slot.size() * h.state.zoom);
        assert!(h.output.shapes.iter().any(|shape| matches!(&shape.shape,
            egui::Shape::Rect(painted) if painted.rect == rect && painted.fill == key_fill)));
    }
    let next = h.screen(keys.slots.last().unwrap().rect.center() + second);
    h.pointer(next, Some(true), Modifiers::NONE);
    h.pointer(next, Some(false), Modifiers::NONE);
    assert_eq!(h.state.pages[&20].1, 1);
    assert_eq!(h.state.pages[&1].1, 0);
    assert!(h.state.selected.is_empty());

    let first_background = h.screen(egui::pos2(25.0, 49.0));
    h.pointer(first_background, Some(true), Modifiers::NONE);
    h.pointer(first_background, Some(false), Modifiers::NONE);
    assert_eq!(h.masks.as_ref().unwrap().focused, Some(1));
    assert_eq!(h.state.softkey_page, 0);
    assert_eq!(h.state.pages[&20].1, 1);
    h.masks.as_mut().unwrap().unfocus();
    h.frame(vec![], Modifiers::NONE);
    let revision = h.project.revision();
    h.key(Key::Delete, Modifiers::NONE);
    assert_eq!(h.project.revision(), revision);
    assert!(h.state.selected.is_empty());
}
