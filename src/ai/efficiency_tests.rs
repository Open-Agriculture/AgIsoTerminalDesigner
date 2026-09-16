use super::*;
use ag_iso_stack::object_pool::{
    object::Object,
    object_attributes::{ObjectRef, Point},
    vt_version::VtVersion,
    ObjectId, ObjectType,
};
use serde_json::json;

pub(super) fn pool(count: u16, text_size: usize) -> AiDesignState {
    let mut state = AiDesignState {
        revision: 7,
        pool: Default::default(),
        object_info: Default::default(),
        vt_version: VtVersion::Version6,
        mask_size: 480,
        selected: Some(ObjectId::new(10).unwrap()),
    };
    let mut ws = crate::default_object(ObjectType::WorkingSet);
    ws.mut_id().set_value(1).unwrap();
    if let Object::WorkingSet(ws) = &mut ws {
        ws.active_mask = ObjectId::new(2).unwrap();
    }
    state.pool.add(ws);
    let mut mask = crate::default_object(ObjectType::DataMask);
    mask.mut_id().set_value(2).unwrap();
    for id in 10..10 + count {
        let mut object = crate::default_object(ObjectType::OutputString);
        object.mut_id().set_value(id).unwrap();
        if let Object::OutputString(v) = &mut object {
            v.value = "x".repeat(text_size);
        }
        state.pool.add(object);
        if let Object::DataMask(v) = &mut mask {
            v.object_refs.push(ObjectRef {
                id: ObjectId::new(id).unwrap(),
                offset: Point { x: 10, y: 20 },
            });
        }
    }
    state.pool.add(mask);
    state
}

#[test]
fn compact_context_pages_every_object_and_keeps_layout_on_later_pages() {
    let state = pool(387, 2000);
    let mut request = DesignContextRequest::default();
    let mut seen = std::collections::HashSet::new();
    loop {
        let result = get_design_context(&state, request.clone()).unwrap();
        assert!(result.objects.len() <= 50);
        assert!(result.layout.len() <= 50);
        assert_eq!(result.total_matching_objects, 388);
        assert!(result
            .objects
            .iter()
            .all(|v| v.properties.is_empty() && v.positioned_children.is_empty()));
        assert!(!result.layout.is_empty());
        for object in result.objects {
            assert!(seen.insert(object.id));
        }
        if let Some(offset) = result.next_offset {
            assert!(result.truncated);
            request.offset = offset;
        } else {
            break;
        }
    }
    assert_eq!(seen.len(), 388);
    let details = get_objects(
        &state,
        GetObjectsRequest {
            ids: vec![10, 11, 10],
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(details.len(), 2);
    assert_eq!(details[0].object.properties["value"], "x".repeat(2000));
    assert!(!details[0].object.parents.is_empty());
    assert!(matches!(
        get_objects(
            &state,
            GetObjectsRequest {
                ids: (10..31).collect(),
                ..Default::default()
            }
        ),
        Err(AiToolError::RequestTooLarge)
    ));
}

#[test]
fn relationship_and_property_pages_are_bounded_and_recoverable() {
    let state = pool(250, 9000);
    let mut request = GetObjectsRequest {
        ids: vec![2],
        include_children: true,
        ..Default::default()
    };
    let mut children = Vec::new();
    loop {
        let result = get_objects(&state, request.clone()).unwrap().remove(0);
        assert!(result.object.positioned_children.len() <= 100);
        children.extend(result.object.positioned_children);
        if let Some(offset) = result.next_detail_offset {
            assert!(result.truncated);
            request.detail_offset = offset;
        } else {
            break;
        }
    }
    assert_eq!(children.len(), 250);
    let mut request = GetObjectsRequest {
        ids: vec![10],
        ..Default::default()
    };
    let mut text = String::new();
    loop {
        let result = get_objects(&state, request.clone()).unwrap().remove(0);
        text.push_str(result.object.properties["value"].as_str().unwrap());
        if let Some(offset) = result.next_property_offset {
            request.property_offset = offset;
        } else {
            break;
        }
    }
    assert_eq!(text, "x".repeat(9000));
    let result = get_objects(
        &state,
        GetObjectsRequest {
            ids: vec![10],
            include_properties: false,
            include_references: false,
            include_parents: false,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(result[0].object.properties.is_empty());
    assert!(result[0].object.references.is_empty());
    assert!(result[0].object.parents.is_empty());
}

#[test]
fn depth_limit_is_explicit_and_subtree_can_be_retrieved() {
    let mut state = pool(0, 0);
    state.selected = None;
    for id in 10..22 {
        let mut object = crate::default_object(ObjectType::Container);
        object.mut_id().set_value(id).unwrap();
        if id < 21 {
            if let Object::Container(v) = &mut object {
                v.object_refs.push(ObjectRef {
                    id: ObjectId::new(id + 1).unwrap(),
                    offset: Point { x: 0, y: 0 },
                });
            }
        }
        state.pool.add(object);
    }
    let first = get_design_context(
        &state,
        DesignContextRequest {
            root_id: Some(10),
            depth: usize::MAX,
            ..Default::default()
        },
    )
    .unwrap();
    assert!(first.depth_limited && first.truncated);
    assert_eq!(first.objects.len(), 9);
    let rest = get_design_context(
        &state,
        DesignContextRequest {
            root_id: Some(18),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(!rest.depth_limited);
    assert_eq!(rest.objects.last().unwrap().id, 21);
}

#[test]
fn usage_aggregation_trace_bounds_and_redaction() {
    let mut debug = debug_trace::AiDebugState::default();
    for total in [1000, 1400, 800] {
        debug.record_usage(AiTokenUsage {
            total_tokens: total,
            ..Default::default()
        });
    }
    assert_eq!(debug.user_request.tokens.total_tokens, 3200);
    assert_eq!(debug.conversation.request_count, 3);
    assert_eq!(debug.last_api.total_tokens, 800);
    debug.begin_request();
    debug.record_usage(AiTokenUsage {
        total_tokens: 100,
        ..Default::default()
    });
    assert_eq!(debug.user_request.tokens.total_tokens, 100);
    assert_eq!(debug.conversation.tokens.total_tokens, 3300);
    for _ in 0..110 {
        debug.trace("request", &json!({"Authorization":"Bearer hidden", "api_key":"hidden", "text":"é".repeat(20000), "image_url":"data:image/png;base64,abc"}));
    }
    assert_eq!(debug.entries.len(), debug_trace::MAX_TRACE_ENTRIES);
    for entry in &debug.entries {
        assert!(entry.truncated);
        assert!(entry.payload.len() <= debug_trace::MAX_RAW_TRACE_BYTES_PER_ENTRY);
        assert!(!entry.payload.contains("hidden"));
        assert!(!entry.payload.contains("base64"));
    }
    let conversation = Conversation::default();
    assert_eq!(
        conversation
            .debug
            .lock()
            .unwrap()
            .conversation
            .tokens
            .total_tokens,
        0
    );
}

#[test]
fn completed_history_drops_tool_chains_and_images_but_keeps_intent() {
    let mut history = Conversation::default();
    for _ in 0..20 {
        history.push(AiMessage::text(AiRole::User, "Make the label wider"));
        history.push(AiMessage {
            role: AiRole::Tool,
            content: vec![
                AiContent::ToolResult(AiToolResult {
                    call_id: "c".into(),
                    result: json!({"objects":"x".repeat(50000)}),
                }),
                AiContent::Image(AiImage {
                    mime_type: "image/png".into(),
                    bytes: vec![42; 10000],
                }),
            ],
        });
    }
    history.push(AiMessage {
        role: AiRole::Tool,
        content: vec![AiContent::ToolResult(AiToolResult {
            call_id: "p".into(),
            result: json!({"proposal_id":"abc", "operation_count":1}),
        })],
    });
    history.compact();
    assert!(history.messages.len() <= 12);
    assert!(history
        .messages
        .iter()
        .flat_map(|m| &m.content)
        .all(|c| matches!(c, AiContent::Text(_))));
    let text = format!("{:?}", history.messages);
    assert!(text.contains("Make the label wider"));
    assert!(text.contains("Visual review"));
    assert!(text.contains("abc"));
    assert!(text.len() < 3000);
}

#[test]
fn repeated_placements_are_available_on_layout_pages() {
    let mut state = pool(1, 0);
    if let Object::DataMask(mask) = state
        .pool
        .object_mut_by_id(ObjectId::new(2).unwrap())
        .unwrap()
    {
        for x in 1..120 {
            mask.object_refs.push(ObjectRef {
                id: ObjectId::new(10).unwrap(),
                offset: Point { x, y: 0 },
            });
        }
    }
    let mut request = DesignContextRequest::default();
    let mut placements = 0;
    loop {
        let result = get_design_context(&state, request.clone()).unwrap();
        assert!(result.layout.len() <= 50);
        placements += result.layout.len();
        if let Some(offset) = result.next_layout_offset {
            assert!(result.truncated);
            request.layout_offset = offset;
        } else {
            break;
        }
    }
    assert_eq!(placements, 120);
}
