//! Isolated state for running a local, no-network VT preview.

use std::{
    collections::{HashMap, HashSet, VecDeque},
    time::Duration,
};

use ag_iso_stack::object_pool::{
    object::{Animation, Object},
    object_attributes::{AnimationSequence, DisabledBehaviour, Event, ObjectRef},
    ObjectId, ObjectPool,
};

use crate::{
    graphics_context_rendering::GraphicsContextCanvas,
    macro_authoring::{decode_macro_commands, MacroInstruction},
    vt_macro_commands::macro_command_targets,
};

/// An executable preview always owns a pool clone, so runtime values cannot
/// leak into the authored project or its undo history.
#[derive(Debug, Clone)]
pub struct PreviewSession {
    pool: ObjectPool,
    version: ag_iso_stack::object_pool::vt_version::VtVersion,
    pixel_limit: usize,
    now: Duration,
    animation: HashMap<u16, AnimationClock>,
    visible_animations: HashSet<u16>,
    animation_null_items: HashSet<(u16, u8)>,
    hidden_objects: HashSet<u16>,
    pressed_buttons: HashSet<u16>,
    selected_input: Option<u16>,
    active_mask: Option<u16>,
    soft_key_mask: Option<u16>,
    graphics: HashMap<u16, GraphicsContextCanvas>,
    macro_stack: Vec<u16>,
    diagnostics: Vec<PreviewDiagnostic>,
    pending_macros: VecDeque<u16>,
    dispatching_events: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreviewDiagnostic {
    pub object_id: u16,
    pub message: String,
}

#[derive(Debug, Clone, Default)]
struct AnimationClock {
    elapsed: Duration,
    last_tick: Option<Duration>,
    updated_at: Option<Duration>,
}

impl PreviewSession {
    pub fn new(authored_pool: &ObjectPool) -> Self {
        Self::with_pixel_limit(
            authored_pool,
            ag_iso_stack::object_pool::vt_version::VtVersion::Version6,
            crate::graphics_context_rendering::default_pixel_limit(),
        )
    }

    pub fn with_version(
        authored_pool: &ObjectPool,
        version: ag_iso_stack::object_pool::vt_version::VtVersion,
    ) -> Self {
        Self::with_pixel_limit(
            authored_pool,
            version,
            crate::graphics_context_rendering::default_pixel_limit(),
        )
    }

    pub fn with_pixel_limit(
        authored_pool: &ObjectPool,
        version: ag_iso_stack::object_pool::vt_version::VtVersion,
        pixel_limit: usize,
    ) -> Self {
        Self {
            pool: authored_pool.clone(),
            version,
            pixel_limit,
            now: Duration::ZERO,
            animation: HashMap::new(),
            visible_animations: HashSet::new(),
            animation_null_items: HashSet::new(),
            hidden_objects: HashSet::new(),
            pressed_buttons: HashSet::new(),
            selected_input: None,
            active_mask: None,
            soft_key_mask: None,
            graphics: HashMap::new(),
            macro_stack: Vec::new(),
            diagnostics: Vec::new(),
            pending_macros: VecDeque::new(),
            dispatching_events: false,
        }
    }

    pub fn pool(&self) -> &ObjectPool {
        &self.pool
    }
    pub fn pool_mut(&mut self) -> &mut ObjectPool {
        &mut self.pool
    }
    pub fn pixel_limit(&self) -> usize {
        self.pixel_limit
    }

    /// Begin an egui frame using its monotonic elapsed time. Call
    /// `finish_frame` after all visible mask objects have been rendered.
    pub fn begin_frame(&mut self, now: Duration) {
        self.now = now;
        self.visible_animations.clear();
    }

    pub fn finish_frame(&mut self) {
        for (id, clock) in &mut self.animation {
            if !self.visible_animations.contains(id) {
                clock.last_tick = None;
            }
        }
    }

    /// Resolve the child currently shown by an Animation. Repeated visible
    /// placements share one clock and one selected frame per object ID.
    pub fn animation_child(&mut self, id: u16) -> Option<ObjectRef> {
        self.visible_animations.insert(id);
        let object = match self.pool.object_by_id(ObjectId::new(id).ok()?)? {
            Object::Animation(animation) => animation.clone(),
            _ => return None,
        };
        let clock = self.animation.entry(id).or_default();
        let mut value = object.value;
        if object.enabled && object.refresh_interval > 0 {
            if clock.updated_at != Some(self.now) {
                if let Some(last) = clock.last_tick {
                    clock.elapsed = clock.elapsed.saturating_add(self.now.saturating_sub(last));
                }
                let interval = Duration::from_millis(u64::from(object.refresh_interval));
                let interval_nanos = interval.as_nanos();
                let steps = clock.elapsed.as_nanos() / interval_nanos;
                if steps > 0 {
                    clock.elapsed =
                        Duration::from_nanos((clock.elapsed.as_nanos() % interval_nanos) as u64);
                    value = advance_animation_index(&object, value, steps);
                }
                clock.updated_at = Some(self.now);
                clock.last_tick = Some(self.now);
            }
            if let Some(Object::Animation(runtime_object)) =
                self.pool.object_mut_by_id(ObjectId::new(id).ok()?)
            {
                runtime_object.value = value;
            }
        }
        let index = if object.enabled {
            value
        } else {
            match object.options.disabled_behaviour {
                DisabledBehaviour::Pause => value,
                DisabledBehaviour::ResetToFirst => object.first_child_index,
                DisabledBehaviour::DefaultObject => object.default_child_index,
                DisabledBehaviour::Blank => return None,
            }
        };
        if self.animation_null_items.contains(&(id, index)) {
            None
        } else {
            animation_child_at(&object, index)
        }
    }

    pub fn set_object_enabled(&mut self, id: u16, enabled: bool) -> Result<(), PreviewDiagnostic> {
        let object_id = ObjectId::new(id).map_err(|e| diag(id, format!("{e:?}")))?;
        let event = if enabled {
            Event::OnEnable
        } else {
            Event::OnDisable
        };
        let mut animation_transition = None;
        let handled = match self.pool.object_mut_by_id(object_id) {
            Some(Object::Animation(object)) => {
                let was_enabled = object.enabled;
                if !enabled && object.options.disabled_behaviour == DisabledBehaviour::ResetToFirst
                {
                    object.value = object.first_child_index;
                }
                if enabled
                    && !was_enabled
                    && object.options.animation_sequence == AnimationSequence::SingleShot
                {
                    object.value = object.first_child_index;
                }
                object.enabled = enabled;
                animation_transition = Some((was_enabled, enabled));
                true
            }
            Some(Object::Button(object)) => {
                object.options.disabled = !enabled;
                true
            }
            Some(Object::InputBoolean(object)) => {
                object.enabled = enabled;
                true
            }
            Some(Object::InputString(object)) => {
                object.enabled = enabled;
                true
            }
            _ => false,
        };
        if !handled {
            return Err(diag(
                id,
                "Enable/Disable Object target is not an enableable preview object",
            ));
        }
        if let Some((was_enabled, enabled)) = animation_transition {
            if was_enabled != enabled {
                let clock = self.animation.entry(id).or_default();
                clock.elapsed = Duration::ZERO;
                clock.updated_at = None;
                clock.last_tick = enabled.then_some(self.now);
            }
        }
        self.trigger_event(id, event)
    }

    pub fn is_hidden(&self, id: u16) -> bool {
        self.hidden_objects.contains(&id)
    }
    pub fn selected_input(&self) -> Option<u16> {
        self.selected_input
    }
    pub fn activate_mask(&mut self, mask_id: u16) -> Result<(), PreviewDiagnostic> {
        let mask_object_id = match ObjectId::new(mask_id) {
            Ok(id) => id,
            Err(error) => return self.fail(mask_id, format!("{error:?}")),
        };
        if !matches!(
            self.pool.object_by_id(mask_object_id),
            Some(Object::DataMask(_) | Object::AlarmMask(_))
        ) {
            return self.fail(
                mask_id,
                "active mask target is not a Data Mask or Alarm Mask",
            );
        }
        let previous = self.active_mask;
        if previous == Some(mask_id) {
            return Ok(());
        }
        let working_set_id = self
            .pool
            .working_set_object()
            .map(|working_set| working_set.id.value());
        if let Some(working_set_id) = working_set_id {
            if let Some(Object::WorkingSet(working_set)) = self
                .pool
                .object_mut_by_id(ObjectId::new(working_set_id).expect("id came from pool"))
            {
                working_set.active_mask = mask_object_id;
            }
        }
        self.active_mask = Some(mask_id);
        self.soft_key_mask = match self.pool.object_by_id(mask_object_id) {
            Some(Object::DataMask(mask)) => mask.soft_key_mask.0.map(|id| id.value()),
            Some(Object::AlarmMask(mask)) => mask.soft_key_mask.0.map(|id| id.value()),
            _ => None,
        };
        if let Some(previous) = previous {
            self.trigger_event(previous, Event::OnHide)?;
        }
        self.trigger_event(mask_id, Event::OnShow)?;
        if let Some(working_set_id) = working_set_id {
            self.trigger_event(working_set_id, Event::OnChangeActiveMask)?;
        }
        Ok(())
    }

    pub fn trigger_event(&mut self, object_id: u16, event: Event) -> Result<(), PreviewDiagnostic> {
        let refs = self
            .pool
            .object_by_id(ObjectId::new(object_id).map_err(|e| diag(object_id, format!("{e:?}")))?)
            .and_then(crate::operations::macro_refs)
            .unwrap_or_default()
            .to_vec();
        for macro_ref in refs
            .into_iter()
            .filter(|reference| reference.event_id == event)
        {
            self.pending_macros.push_back(macro_ref.macro_id);
        }
        if self.macro_stack.is_empty() && !self.dispatching_events {
            self.drain_event_macros()?;
        }
        Ok(())
    }
    pub fn trigger_key_event(
        &mut self,
        object_id: u16,
        pressed: bool,
    ) -> Result<(), PreviewDiagnostic> {
        let is_latchable = match ObjectId::new(object_id)
            .ok()
            .and_then(|id| self.pool.object_by_id(id))
        {
            Some(Object::Button(button)) => button.options.latchable,
            Some(Object::Key(_)) => false,
            _ => return self.fail(object_id, "key event target is not a Button or Key"),
        };
        if is_latchable {
            if pressed && !self.pressed_buttons.insert(object_id) {
                self.pressed_buttons.remove(&object_id);
            }
        } else if pressed {
            self.pressed_buttons.insert(object_id);
        } else {
            self.pressed_buttons.remove(&object_id);
        }
        self.trigger_event(
            object_id,
            if pressed {
                Event::OnKeyPress
            } else {
                Event::OnKeyRelease
            },
        )
    }
    pub fn button_is_pressed(&self, object_id: u16) -> bool {
        self.pressed_buttons.contains(&object_id)
    }
    pub fn select_input(&mut self, object_id: u16) {
        self.selected_input = Some(object_id);
    }
    pub fn show_object(&mut self, object_id: u16, show: bool) {
        if show {
            self.hidden_objects.remove(&object_id);
        } else {
            self.hidden_objects.insert(object_id);
        }
    }
    pub fn active_mask(&self) -> Option<u16> {
        self.active_mask
    }
    pub fn soft_key_mask(&self) -> Option<u16> {
        self.soft_key_mask
    }

    pub fn graphics_context(&mut self, id: u16) -> Option<GraphicsContextCanvas> {
        if !self.graphics.contains_key(&id) {
            let object = match self.pool.object_by_id(ObjectId::new(id).ok()?)? {
                Object::GraphicsContext(object) => object.clone(),
                _ => return None,
            };
            match GraphicsContextCanvas::try_from_object(&object, self.pixel_limit) {
                Ok(canvas) => {
                    self.graphics.insert(id, canvas);
                }
                Err(message) => {
                    self.diagnostics.push(diag(id, message));
                    return None;
                }
            }
        }
        self.graphics.get(&id).cloned()
    }

    pub fn graphics_context_mut(&mut self, id: u16) -> Option<&mut GraphicsContextCanvas> {
        if !self.graphics.contains_key(&id) {
            self.graphics_context(id)?;
        }
        self.graphics.get_mut(&id)
    }

    pub fn diagnostics(&self) -> &[PreviewDiagnostic] {
        &self.diagnostics
    }
    pub fn take_diagnostics(&mut self) -> Vec<PreviewDiagnostic> {
        std::mem::take(&mut self.diagnostics)
    }

    pub fn reset(&mut self, authored_pool: &ObjectPool) {
        *self = Self::with_pixel_limit(authored_pool, self.version, self.pixel_limit);
    }

    pub fn execute_macro(&mut self, id: u16) -> Result<(), PreviewDiagnostic> {
        let result = self.execute_macro_inner(id);
        if let Err(error) = &result {
            self.record_diagnostic(error.clone());
            if self.macro_stack.is_empty() {
                self.pending_macros.clear();
            }
        }
        result
    }

    fn execute_macro_inner(&mut self, id: u16) -> Result<(), PreviewDiagnostic> {
        if self.macro_stack.len() >= 64 || self.macro_stack.contains(&id) {
            return self.fail(id, "recursive Macro execution stopped in local preview");
        }
        let Some(Object::Macro(object)) = self
            .pool
            .object_by_id(ObjectId::new(id).map_err(|e| diag(id, format!("{e:?}")))?)
            .cloned()
        else {
            return self.fail(
                id,
                "Execute Macro target does not resolve to a Macro object",
            );
        };
        let instructions = decode_macro_commands(&object.commands, self.version)
            .map_err(|message| diag(id, message))?;
        for target in macro_command_targets(&object.commands, self.version)
            .map_err(|message| diag(id, message))?
        {
            let Some(target_id) = target.object_id else {
                if !target.nullable {
                    return self.fail(
                        id,
                        format!(
                            "Macro command at offset {} has a null object operand",
                            target.command_offset
                        ),
                    );
                }
                continue;
            };
            let Some(target_object) = ObjectId::new(target_id)
                .ok()
                .and_then(|object_id| self.pool.object_by_id(object_id))
            else {
                return self.fail(
                    id,
                    format!(
                        "Macro command at offset {} targets missing object {target_id}",
                        target.command_offset
                    ),
                );
            };
            if !target.expected_types.is_empty()
                && !target.expected_types.contains(&target_object.object_type())
            {
                return self.fail(
                    id,
                    format!(
                        "Macro command at offset {} targets object {target_id} with wrong type {:?}",
                        target.command_offset,
                        target_object.object_type()
                    ),
                );
            }
        }
        self.macro_stack.push(id);
        let result = instructions
            .iter()
            .try_for_each(|instruction| self.execute_instruction(id, instruction));
        self.macro_stack.pop();
        if result.is_ok() && self.macro_stack.is_empty() && !self.dispatching_events {
            self.drain_event_macros()?;
        }
        result
    }

    fn drain_event_macros(&mut self) -> Result<(), PreviewDiagnostic> {
        const MAX_EVENT_MACROS_PER_DISPATCH: usize = 4096;
        self.dispatching_events = true;
        let mut executed = 0;
        while let Some(macro_id) = self.pending_macros.pop_front() {
            executed += 1;
            if executed > MAX_EVENT_MACROS_PER_DISPATCH {
                self.pending_macros.clear();
                self.dispatching_events = false;
                return self.fail(
                    macro_id,
                    "event-triggered Macro loop stopped after 4096 executions",
                );
            }
            if let Err(error) = self.execute_macro(macro_id) {
                self.pending_macros.clear();
                self.dispatching_events = false;
                return Err(error);
            }
        }
        self.dispatching_events = false;
        Ok(())
    }

    fn execute_instruction(
        &mut self,
        source: u16,
        instruction: &MacroInstruction,
    ) -> Result<(), PreviewDiagnostic> {
        let args = &instruction.arguments;
        let parse_id = |index: usize| {
            args.get(index)
                .and_then(|s| s.parse::<u16>().ok())
                .ok_or_else(|| {
                    diag(
                        source,
                        format!("invalid {} target in {}", index + 1, instruction.name),
                    )
                })
        };
        match instruction.name.as_str() {
            "execute-macro" => self.execute_macro(
                args.first()
                    .and_then(|s| s.parse().ok())
                    .ok_or_else(|| diag(source, "invalid Execute Macro operand"))?,
            ),
            "hide-show-object" => {
                let id = parse_id(0)?;
                let event = match args.get(1).map(String::as_str) {
                    Some("hide") => {
                        self.hidden_objects.insert(id);
                        Event::OnHide
                    }
                    Some("show") => {
                        self.hidden_objects.remove(&id);
                        Event::OnShow
                    }
                    _ => return self.fail(source, "invalid Hide/Show Object mode"),
                };
                self.trigger_event(id, event)
            }
            "enable-disable-object" => {
                let id = parse_id(0)?;
                match args.get(1).map(String::as_str) {
                    Some("disable") => self.set_object_enabled(id, false),
                    Some("enable") => self.set_object_enabled(id, true),
                    _ => self.fail(source, "invalid Enable/Disable Object mode"),
                }
            }
            "select-input-object" => {
                self.selected_input = Some(parse_id(0)?);
                Ok(())
            }
            "change-active-mask" => {
                parse_id(0)?;
                match nullable_id(args.get(1)).unwrap_or(None) {
                    Some(mask_id) => self.activate_mask(mask_id),
                    None => {
                        if let Some(previous) = self.active_mask.take() {
                            self.trigger_event(previous, Event::OnHide)?;
                        }
                        Ok(())
                    }
                }
            }
            "change-soft-key-mask" => {
                let expected_type = match args.first().map(String::as_str) {
                    Some("data") => ag_iso_stack::object_pool::ObjectType::DataMask,
                    Some("alarm") => ag_iso_stack::object_pool::ObjectType::AlarmMask,
                    _ => return self.fail(source, "invalid Change Soft Key Mask mask type"),
                };
                let mask_id = nullable_id(args.get(1)).unwrap_or(None);
                let key_mask = nullable_id(args.get(2)).unwrap_or(None);
                let Some(mask_id) = mask_id else {
                    return self.fail(source, "Change Soft Key Mask requires a mask object");
                };
                let mask_object_id =
                    ObjectId::new(mask_id).map_err(|e| diag(source, format!("{e:?}")))?;
                if self
                    .pool
                    .object_by_id(mask_object_id)
                    .map(Object::object_type)
                    != Some(expected_type)
                {
                    return self.fail(
                        source,
                        "Change Soft Key Mask target has the wrong mask type",
                    );
                }
                let updated = match self.pool.object_mut_by_id(mask_object_id) {
                    Some(Object::DataMask(mask)) => {
                        mask.soft_key_mask = key_mask
                            .map_or(ag_iso_stack::object_pool::NullableObjectId::NULL, |id| {
                                ag_iso_stack::object_pool::NullableObjectId::new(id)
                            });
                        true
                    }
                    Some(Object::AlarmMask(mask)) => {
                        mask.soft_key_mask = key_mask
                            .map_or(ag_iso_stack::object_pool::NullableObjectId::NULL, |id| {
                                ag_iso_stack::object_pool::NullableObjectId::new(id)
                            });
                        true
                    }
                    _ => false,
                };
                if !updated {
                    return self.fail(
                        source,
                        "Change Soft Key Mask target is not a Data Mask or Alarm Mask",
                    );
                }
                if self.active_mask == Some(mask_id) {
                    self.soft_key_mask = key_mask;
                }
                self.trigger_event(mask_id, Event::OnChangeSoftKeyMask)
            }
            "change-numeric-value" => self.set_numeric_value(
                parse_id(0)?,
                args.get(1)
                    .and_then(|s| s.parse::<u32>().ok())
                    .ok_or_else(|| diag(source, "invalid Change Numeric Value operand"))?,
            ),
            "change-background-colour" => self.set_background_colour(
                parse_id(0)?,
                args.get(1)
                    .and_then(|s| s.parse::<u8>().ok())
                    .ok_or_else(|| diag(source, "invalid background colour"))?,
            ),
            "change-size" => self.set_object_size(
                parse_id(0)?,
                args.get(1)
                    .and_then(|s| s.parse::<u16>().ok())
                    .ok_or_else(|| diag(source, "invalid object width"))?,
                args.get(2)
                    .and_then(|s| s.parse::<u16>().ok())
                    .ok_or_else(|| diag(source, "invalid object height"))?,
            ),
            "change-child-position" => self.set_child_position(
                parse_id(0)?,
                parse_id(1)?,
                args.get(2)
                    .and_then(|s| s.parse::<i16>().ok())
                    .ok_or_else(|| diag(source, "invalid child X position"))?,
                args.get(3)
                    .and_then(|s| s.parse::<i16>().ok())
                    .ok_or_else(|| diag(source, "invalid child Y position"))?,
                false,
            ),
            "change-child-location" => self.set_child_position(
                parse_id(0)?,
                parse_id(1)?,
                args.get(2)
                    .and_then(|s| s.parse::<i16>().ok())
                    .ok_or_else(|| diag(source, "invalid child X location"))?,
                args.get(3)
                    .and_then(|s| s.parse::<i16>().ok())
                    .ok_or_else(|| diag(source, "invalid child Y location"))?,
                true,
            ),
            "change-string-value" | "change-string-value-wide" => self.set_string_value(
                parse_id(0)?,
                args.get(1)
                    .cloned()
                    .ok_or_else(|| diag(source, "missing string value"))?,
            ),
            "graphics-context" => self.execute_graphics_context(source, args),
            "control-audio-signal" | "set-audio-volume" => {
                self.record_diagnostic(diag(
                    source,
                    "Audio commands are not played by the local preview",
                ));
                Ok(())
            }
            "lock-unlock-mask" => Ok(()),
            "change-list-item"
                if matches!(
                    ObjectId::new(parse_id(0)?)
                        .ok()
                        .and_then(|id| self.pool.object_by_id(id)),
                    Some(Object::Animation(_))
                ) =>
            {
                let id = parse_id(0)?;
                let index = args
                    .get(1)
                    .and_then(|value| value.parse::<u8>().ok())
                    .ok_or_else(|| diag(source, "invalid Animation list index"))?;
                let Some(Object::Animation(animation)) = self
                    .pool
                    .object_by_id(ObjectId::new(id).map_err(|e| diag(source, format!("{e:?}")))?)
                else {
                    unreachable!();
                };
                if usize::from(index) >= animation.object_refs.len() {
                    return self.fail(source, "Animation list index is outside the object list");
                }
                if nullable_id(args.get(2)).flatten().is_none() {
                    self.animation_null_items.insert((id, index));
                } else {
                    crate::preview_commands::apply(&mut self.pool, instruction, self.version)
                        .map_err(|message| diag(source, message))?;
                    self.animation_null_items.remove(&(id, index));
                }
                Ok(())
            }
            _ => match crate::preview_commands::apply(&mut self.pool, instruction, self.version) {
                Ok(events) => {
                    for (id, canvas) in &mut self.graphics {
                        if let Some(Object::GraphicsContext(object)) = ObjectId::new(*id)
                            .ok()
                            .and_then(|id| self.pool.object_by_id(id))
                        {
                            canvas.update_attributes(object);
                        }
                    }
                    for (object_id, event) in events {
                        self.trigger_event(object_id, event)?;
                    }
                    Ok(())
                }
                Err(message) => self.fail(source, message),
            },
        }
    }

    pub fn set_numeric_value(&mut self, id: u16, value: u32) -> Result<(), PreviewDiagnostic> {
        let object_id = ObjectId::new(id).map_err(|e| diag(id, format!("{e:?}")))?;
        match self.pool.object_mut_by_id(object_id) {
            Some(Object::Animation(animation)) => {
                animation.value = value as u8;
                self.animation.entry(id).or_default().elapsed = Duration::ZERO;
                self.animation.entry(id).or_default().last_tick = Some(self.now);
            }
            Some(Object::NumberVariable(variable)) => {
                variable.value = value;
            }
            Some(Object::InputNumber(object)) => object.value = value,
            Some(Object::OutputNumber(object)) => object.value = value,
            Some(Object::InputBoolean(object)) => object.value = value as u8,
            Some(Object::InputList(object)) => object.value = value as u8,
            Some(Object::OutputList(object)) => object.value = value as u8,
            Some(Object::OutputMeter(object)) => object.value = value as u16,
            Some(Object::OutputLinearBarGraph(object)) => object.value = value as u16,
            Some(Object::OutputArchedBarGraph(object)) => object.value = value as u16,
            Some(Object::ObjectPointer(object)) => {
                object.value = ag_iso_stack::object_pool::NullableObjectId::new(value as u16)
            }
            Some(Object::ExternalObjectPointer(object)) => {
                object.external_object_id =
                    ag_iso_stack::object_pool::NullableObjectId::new(value as u16)
            }
            Some(Object::ScaledGraphic(object)) => {
                object.value = ag_iso_stack::object_pool::NullableObjectId::new(value as u16)
            }
            _ => {
                return Err(diag(
                    id,
                    "Change Numeric Value target is not a supported numeric object",
                ))
            }
        }
        self.trigger_event(id, Event::OnChangeValue)
    }

    pub fn set_background_colour(&mut self, id: u16, colour: u8) -> Result<(), PreviewDiagnostic> {
        let object_id = ObjectId::new(id).map_err(|e| diag(id, format!("{e:?}")))?;
        match self.pool.object_mut_by_id(object_id) {
            Some(Object::WorkingSet(object)) => object.background_colour = colour,
            Some(Object::DataMask(object)) => object.background_colour = colour,
            Some(Object::AlarmMask(object)) => object.background_colour = colour,
            Some(Object::Key(object)) => object.background_colour = colour,
            Some(Object::Button(object)) => object.background_colour = colour,
            Some(Object::InputBoolean(object)) => object.background_colour = colour,
            Some(Object::InputString(object)) => object.background_colour = colour,
            Some(Object::OutputString(object)) => object.background_colour = colour,
            Some(Object::OutputNumber(object)) => object.background_colour = colour,
            Some(Object::GraphicsContext(object)) => object.background_colour = colour,
            _ => {
                return Err(diag(
                    id,
                    "Change Background Colour target is unsupported in preview",
                ))
            }
        }
        if let Some(canvas) = self.graphics.get_mut(&id) {
            canvas.clear(colour);
        }
        self.trigger_event(id, Event::OnChangeBackgroundColour)
    }

    pub fn set_object_size(
        &mut self,
        id: u16,
        width: u16,
        height: u16,
    ) -> Result<(), PreviewDiagnostic> {
        let object_id = ObjectId::new(id).map_err(|e| diag(id, format!("{e:?}")))?;
        let resized = match self.pool.object_mut_by_id(object_id) {
            Some(Object::Button(object)) => {
                object.width = width;
                object.height = height;
                true
            }
            Some(Object::InputString(object)) => {
                object.width = width;
                object.height = height;
                true
            }
            Some(Object::InputNumber(object)) => {
                object.width = width;
                object.height = height;
                true
            }
            Some(Object::InputList(object)) => {
                object.width = width;
                object.height = height;
                true
            }
            Some(Object::OutputString(object)) => {
                object.width = width;
                object.height = height;
                true
            }
            Some(Object::OutputNumber(object)) => {
                object.width = width;
                object.height = height;
                true
            }
            Some(Object::OutputList(object)) => {
                object.width = width;
                object.height = height;
                true
            }
            Some(Object::OutputRectangle(object)) => {
                object.width = width;
                object.height = height;
                true
            }
            Some(Object::OutputEllipse(object)) => {
                object.width = width;
                object.height = height;
                true
            }
            Some(Object::OutputPolygon(object)) => {
                object.width = width;
                object.height = height;
                true
            }
            Some(Object::OutputLinearBarGraph(object)) => {
                object.width = width;
                object.height = height;
                true
            }
            Some(Object::OutputArchedBarGraph(object)) => {
                object.width = width;
                object.height = height;
                true
            }
            Some(Object::Animation(object)) => {
                object.width = width;
                object.height = height;
                true
            }
            Some(Object::ScaledGraphic(object)) => {
                object.width = width;
                object.height = height;
                true
            }
            Some(Object::GraphicsContext(object)) => {
                object.viewport_width = width;
                object.viewport_height = height;
                true
            }
            _ => false,
        };
        if !resized {
            return Err(diag(
                id,
                "Change Size target is not a resizable preview object",
            ));
        }
        if let Some(canvas) = self.graphics.get_mut(&id) {
            canvas.viewport.2 = width;
            canvas.viewport.3 = height;
        }
        self.trigger_event(id, Event::OnChangeSize)
    }

    fn set_child_position(
        &mut self,
        parent_id: u16,
        child_id: u16,
        x: i16,
        y: i16,
        relative: bool,
    ) -> Result<(), PreviewDiagnostic> {
        let parent_id_object =
            ObjectId::new(parent_id).map_err(|e| diag(parent_id, format!("{e:?}")))?;
        let mut found = false;
        let refs = match self.pool.object_mut_by_id(parent_id_object) {
            Some(Object::WorkingSet(object)) => Some(&mut object.object_refs),
            Some(Object::DataMask(object)) => Some(&mut object.object_refs),
            Some(Object::AlarmMask(object)) => Some(&mut object.object_refs),
            Some(Object::Container(object)) => Some(&mut object.object_refs),
            Some(Object::Button(object)) => Some(&mut object.object_refs),
            Some(Object::AuxiliaryFunctionType1(object)) => Some(&mut object.object_refs),
            Some(Object::AuxiliaryInputType1(object)) => Some(&mut object.object_refs),
            Some(Object::AuxiliaryFunctionType2(object)) => Some(&mut object.object_refs),
            Some(Object::AuxiliaryInputType2(object)) => Some(&mut object.object_refs),
            Some(Object::WindowMask(object)) => Some(&mut object.object_refs),
            Some(Object::Animation(object)) => Some(&mut object.object_refs),
            _ => None,
        };
        if let Some(refs) = refs {
            if let Some(reference) = refs
                .iter_mut()
                .find(|reference| reference.id.value() == child_id)
            {
                if relative {
                    reference.offset.x = reference.offset.x.saturating_add(x);
                    reference.offset.y = reference.offset.y.saturating_add(y);
                } else {
                    reference.offset.x = x;
                    reference.offset.y = y;
                }
                found = true;
            }
        }
        if !found {
            return Err(diag(
                parent_id,
                format!("parent {parent_id} has no positioned child {child_id}"),
            ));
        }
        self.trigger_event(
            parent_id,
            if relative {
                Event::OnChangeChildLocation
            } else {
                Event::OnChangeChildPosition
            },
        )
    }

    fn set_string_value(&mut self, id: u16, value: String) -> Result<(), PreviewDiagnostic> {
        let object_id = ObjectId::new(id).map_err(|e| diag(id, format!("{e:?}")))?;
        match self.pool.object_mut_by_id(object_id) {
            Some(Object::InputString(object)) => object.value = value.into(),
            Some(Object::OutputString(object)) => object.value = value.into(),
            Some(Object::StringVariable(object)) => object.value = value.into(),
            _ => {
                return Err(diag(
                    id,
                    "Change String Value target is not a string object",
                ))
            }
        }
        self.trigger_event(id, Event::OnChangeValue)
    }

    fn execute_graphics_context(
        &mut self,
        source: u16,
        args: &[String],
    ) -> Result<(), PreviewDiagnostic> {
        let id = args
            .first()
            .and_then(|s| s.parse::<u16>().ok())
            .ok_or_else(|| diag(source, "invalid Graphics Context target"))?;
        let command = args.get(1).map(String::as_str).unwrap_or("");
        let pool_snapshot = self.pool.clone();
        let version = self.version;
        let pixel_limit = self.pixel_limit;
        if self.graphics_context(id).is_none() {
            return self.fail(source, "Graphics Context target does not resolve");
        }
        if matches!(
            command,
            "copy-canvas-to-picture-graphic" | "copy-viewport-to-picture-graphic"
        ) {
            let target_id = args
                .get(2)
                .and_then(|value| value.parse::<u16>().ok())
                .ok_or_else(|| diag(source, "invalid Picture Graphic target"))?;
            let Some(canvas) = self.graphics.get(&id).cloned() else {
                return self.fail(source, "Graphics Context canvas is unavailable");
            };
            let Some(Object::PictureGraphic(picture)) = ObjectId::new(target_id)
                .ok()
                .and_then(|target_id| self.pool.object_mut_by_id(target_id))
            else {
                return self.fail(source, "copy target is not a Picture Graphic");
            };
            copy_to_picture(
                &canvas,
                picture,
                command == "copy-viewport-to-picture-graphic",
                pixel_limit,
            )
            .map_err(|message| diag(source, message))?;
            return Ok(());
        }
        let foreground = {
            let canvas = self.graphics.get(&id).expect("initialized above");
            if canvas.use_attribute_colours {
                canvas
                    .line_attributes
                    .and_then(|id| ObjectId::new(id).ok())
                    .and_then(|id| pool_snapshot.object_by_id(id))
                    .and_then(|object| match object {
                        Object::LineAttributes(line) => Some(line.line_colour),
                        _ => None,
                    })
                    .unwrap_or(canvas.foreground)
            } else {
                canvas.foreground
            }
        };
        let canvas = self
            .graphics_context_mut(id)
            .ok_or_else(|| diag(source, "Graphics Context target does not resolve"))?;
        let number = |i: usize| {
            args.get(i)
                .and_then(|s| s.parse::<i32>().ok())
                .ok_or_else(|| diag(source, format!("invalid {command} operand")))
        };
        match command {
            "set-cursor" => canvas.cursor = (number(2)? as i16, number(3)? as i16),
            "move-cursor" => {
                canvas.cursor = (
                    canvas.cursor.0.saturating_add(number(2)? as i16),
                    canvas.cursor.1.saturating_add(number(3)? as i16),
                )
            }
            "set-foreground-colour" => canvas.foreground = number(2)? as u8,
            "set-background-colour" => {
                canvas.background = number(2)? as u8;
            }
            "set-line-attributes" => {
                canvas.line_attributes = nullable_id(args.get(2)).unwrap_or(None)
            }
            "set-fill-attributes" => {
                canvas.fill_attributes = nullable_id(args.get(2)).unwrap_or(None)
            }
            "set-font-attributes" => {
                canvas.font_attributes = nullable_id(args.get(2)).unwrap_or(None)
            }
            "erase-rectangle" => {
                let (x, y) = (i32::from(canvas.cursor.0), i32::from(canvas.cursor.1));
                let (width, height) = (number(2)?.max(0) as u16, number(3)?.max(0) as u16);
                canvas.fill_rect(x, y, width, height, canvas.background);
                if width > 0 && height > 0 {
                    canvas.cursor = (
                        (x + i32::from(width) - 1) as i16,
                        (y + i32::from(height) - 1) as i16,
                    );
                }
            }
            "draw-point" => {
                let x = canvas.cursor.0.saturating_add(number(2)? as i16);
                let y = canvas.cursor.1.saturating_add(number(3)? as i16);
                canvas.pixel(i32::from(x), i32::from(y), foreground);
                canvas.cursor = (x, y);
            }
            "draw-line" | "draw-rectangle" | "draw-closed-ellipse" | "draw-polygon" => {
                let start = (i32::from(canvas.cursor.0), i32::from(canvas.cursor.1));
                let mut line = match canvas
                    .line_attributes
                    .and_then(|id| ObjectId::new(id).ok())
                    .and_then(|id| pool_snapshot.object_by_id(id))
                {
                    Some(Object::LineAttributes(line)) => line.clone(),
                    _ => {
                        let Object::LineAttributes(mut line) = crate::default_object(
                            ag_iso_stack::object_pool::ObjectType::LineAttributes,
                        ) else {
                            unreachable!();
                        };
                        line.line_width = 0;
                        line
                    }
                };
                if !canvas.use_attribute_colours {
                    line.line_colour = canvas.foreground;
                }
                let mut fill = canvas
                    .fill_attributes
                    .and_then(|id| ObjectId::new(id).ok())
                    .and_then(|id| pool_snapshot.object_by_id(id))
                    .cloned();
                if !canvas.use_attribute_colours {
                    if let Some(Object::FillAttributes(fill)) = &mut fill {
                        fill.fill_colour = canvas.background;
                    }
                }
                let brush = f32::from(line.line_width) * 0.5;
                let point = |x: i32, y: i32| eframe::egui::pos2(x as f32 + brush, y as f32 + brush);
                let mut end = start;
                let points = match command {
                    "draw-line" => {
                        end = (start.0 + number(2)?, start.1 + number(3)?);
                        fill = None;
                        vec![point(start.0, start.1), point(end.0, end.1)]
                    }
                    "draw-polygon" => {
                        let count = number(2)?.max(0) as usize;
                        let mut points = vec![point(start.0, start.1)];
                        for i in 0..count {
                            end = (start.0 + number(3 + i * 2)?, start.1 + number(4 + i * 2)?);
                            points.push(point(end.0, end.1));
                        }
                        if end != start {
                            fill = None;
                        }
                        points
                    }
                    _ => {
                        let width = number(2)?.max(0) as f32;
                        let height = number(3)?.max(0) as f32;
                        if width == 0.0 || height == 0.0 {
                            return Ok(());
                        }
                        end = (start.0 + width as i32 - 1, start.1 + height as i32 - 1);
                        let rect = eframe::egui::Rect::from_min_size(
                            eframe::egui::pos2(start.0 as f32, start.1 as f32),
                            eframe::egui::vec2(width, height),
                        );
                        if command == "draw-closed-ellipse" {
                            crate::object_rendering::ellipse_path(
                                rect.shrink(brush.min((width.min(height) * 0.5 - 0.001).max(0.0))),
                                0,
                                0,
                                0,
                            )
                        } else {
                            let min = rect.min + eframe::egui::vec2(brush, brush);
                            let max = rect.min
                                + eframe::egui::vec2(
                                    (width - f32::from(line.line_width)).max(0.0) + brush,
                                    (height - f32::from(line.line_width)).max(0.0) + brush,
                                );
                            vec![
                                min,
                                eframe::egui::pos2(max.x, min.y),
                                max,
                                eframe::egui::pos2(min.x, max.y),
                                min,
                            ]
                        }
                    }
                };
                if canvas.width > 0 && canvas.height > 0 {
                    let raster = crate::graphics_context_raster::rasterize_paint(
                        canvas.width,
                        canvas.height,
                        pixel_limit,
                        |ui| {
                            crate::object_rendering::paint_shape(
                                ui,
                                &pool_snapshot,
                                ui.clip_rect(),
                                &points,
                                &line,
                                fill.as_ref(),
                                0,
                            );
                        },
                    )
                    .map_err(|message| diag(source, message))?;
                    composite_raster(canvas, &raster, 0, 0, &pool_snapshot);
                }
                canvas.cursor = (end.0 as i16, end.1 as i16);
            }
            "draw-text" => {
                let text = args.get(3).cloned().unwrap_or_default();
                let transparent = match args.get(2).map(String::as_str) {
                    Some("transparent") => true,
                    Some("opaque") => false,
                    _ => return self.fail(source, "invalid Graphics Context Draw Text mode"),
                };
                let font_id = canvas
                    .font_attributes
                    .and_then(|id| ObjectId::new(id).ok())
                    .ok_or_else(|| diag(source, "Draw Text requires Font Attributes"))?;
                let mut display_pool = pool_snapshot.clone();
                let Some(Object::FontAttributes(font)) = display_pool.object_mut_by_id(font_id)
                else {
                    return Err(diag(source, "Draw Text Font Attributes are missing"));
                };
                font.font_style.flashing_hidden = false;
                font.font_style.flashing_inverted = false;
                if !canvas.use_attribute_colours {
                    font.font_colour = canvas.foreground;
                }
                let value = ag_iso_stack::object_pool::object::VtString::Encoded {
                    encoded_bytes: text
                        .chars()
                        .map(|c| {
                            u8::try_from(c as u32).map_err(|_| {
                                diag(source, "Draw Text contains an invalid encoded byte")
                            })
                        })
                        .collect::<Result<_, _>>()?,
                };
                let decoded = value
                    .decode(font.font_type)
                    .map_err(|e| diag(source, e.to_string()))?;
                let ctx = eframe::egui::Context::default();
                let mut extent = eframe::egui::Vec2::ZERO;
                let _ = ctx.run(Default::default(), |ctx| {
                    eframe::egui::CentralPanel::default().show(ctx, |ui| {
                        extent = crate::output_string_rendering::text_extent(ui, font, &decoded);
                    });
                });
                let width = extent.x.ceil().clamp(0.0, u16::MAX as f32) as u16;
                let height = extent.y.ceil().clamp(0.0, u16::MAX as f32) as u16;
                if width > 0 && height > 0 {
                    let mut object = crate::object_defaults::default_object(
                        ag_iso_stack::object_pool::ObjectType::OutputString,
                    );
                    let Object::OutputString(output) = &mut object else {
                        unreachable!();
                    };
                    output.width = width;
                    output.height = height;
                    output.background_colour = canvas.background;
                    output.font_attributes = font_id.into();
                    output.options.transparent = transparent;
                    output.options.auto_wrap = false;
                    output.value = value;
                    let raster = crate::graphics_context_raster::rasterize(
                        &display_pool,
                        &object,
                        width,
                        height,
                        version,
                        (60, 60),
                        pixel_limit,
                    )
                    .map_err(|message| diag(source, message))?;
                    let (x, y) = (i32::from(canvas.cursor.0), i32::from(canvas.cursor.1));
                    composite_raster(canvas, &raster, x, y, &display_pool);
                    canvas.cursor = (
                        (x + i32::from(width) - 1) as i16,
                        (y + i32::from(height) - 1) as i16,
                    );
                }
            }
            "draw-vt-object" => {
                let target_id = args
                    .get(2)
                    .and_then(|value| value.parse::<u16>().ok())
                    .ok_or_else(|| diag(source, "invalid Draw VT Object target"))?;
                if crate::pool_validation::would_create_circular_reference(
                    &pool_snapshot,
                    ObjectId::new(id).map_err(|e| diag(source, format!("{e:?}")))?,
                    ObjectId::new(target_id).map_err(|e| diag(source, format!("{e:?}")))?,
                )
                .is_err()
                {
                    return self.fail(
                        source,
                        "Draw VT Object cannot contain the destination Graphics Context",
                    );
                }
                let mut display_pool = pool_snapshot.clone();
                for object in display_pool.objects_mut() {
                    if let Object::PictureGraphic(picture) = object {
                        picture.options.flashing = false;
                    }
                }
                let target = ObjectId::new(target_id)
                    .ok()
                    .and_then(|target_id| display_pool.object_by_id(target_id))
                    .ok_or_else(|| diag(source, "Draw VT Object target does not resolve"))?;
                let sized = target
                    .as_sized_object()
                    .ok_or_else(|| diag(source, "Draw VT Object target has no preview size"))?;
                let (width, height) = (sized.width(), sized.height());
                let raster = crate::graphics_context_raster::rasterize(
                    &display_pool,
                    target,
                    width,
                    height,
                    version,
                    (0, 0),
                    pixel_limit,
                )
                .map_err(|message| diag(source, message))?;
                let (x, y) = (canvas.cursor.0.into(), canvas.cursor.1.into());
                composite_raster(canvas, &raster, x, y, &display_pool);
                canvas.cursor = (
                    x.saturating_add(i32::from(width).saturating_sub(1)) as i16,
                    y.saturating_add(i32::from(height).saturating_sub(1)) as i16,
                );
            }
            "pan-viewport" => {
                canvas.viewport.0 = number(2)? as i16;
                canvas.viewport.1 = number(3)? as i16;
            }
            "zoom-viewport" => {
                let z = args
                    .get(2)
                    .and_then(|s| s.parse::<f32>().ok())
                    .ok_or_else(|| diag(source, "invalid viewport zoom"))?;
                if z.is_finite() && z > 0.0 && z <= 32.0 {
                    canvas.zoom = z;
                } else {
                    return self.fail(source, "viewport zoom must be finite and positive");
                }
            }
            "pan-and-zoom-viewport" => {
                canvas.viewport.0 = number(2)? as i16;
                canvas.viewport.1 = number(3)? as i16;
                let z = args
                    .get(4)
                    .and_then(|s| s.parse::<f32>().ok())
                    .ok_or_else(|| diag(source, "invalid viewport zoom"))?;
                if z.is_finite() && z > 0.0 && z <= 32.0 {
                    canvas.zoom = z;
                } else {
                    return self.fail(source, "viewport zoom must be finite and positive");
                }
            }
            "change-viewport-size" => {
                canvas.viewport.2 = number(2)?.max(0) as u16;
                canvas.viewport.3 = number(3)?.max(0) as u16;
            }
            name => {
                return self.fail(
                    source,
                    format!("Graphics Context {name} is not supported in local preview"),
                )
            }
        }
        if let Some(Object::GraphicsContext(object)) = ObjectId::new(id)
            .ok()
            .and_then(|id| self.pool.object_mut_by_id(id))
        {
            if let Some(canvas) = self.graphics.get(&id) {
                canvas.write_attributes(object);
            }
        }
        Ok(())
    }

    fn fail<T>(&mut self, id: u16, message: impl Into<String>) -> Result<T, PreviewDiagnostic> {
        let error = diag(id, message);
        self.record_diagnostic(error.clone());
        Err(error)
    }

    fn record_diagnostic(&mut self, diagnostic: PreviewDiagnostic) {
        if self.diagnostics.last() != Some(&diagnostic) {
            self.diagnostics.push(diagnostic);
        }
    }
}

fn diag(object_id: u16, message: impl Into<String>) -> PreviewDiagnostic {
    PreviewDiagnostic {
        object_id,
        message: message.into(),
    }
}

fn composite_raster(
    canvas: &mut GraphicsContextCanvas,
    raster: &eframe::egui::ColorImage,
    x: i32,
    y: i32,
    pool: &ObjectPool,
) {
    let palette = crate::effective_colour::colours(pool);
    let mut colour_cache = HashMap::new();
    for source_y in 0..raster.size[1] {
        for source_x in 0..raster.size[0] {
            let destination_x = x + source_x as i32;
            let destination_y = y + source_y as i32;
            if destination_x < 0
                || destination_y < 0
                || destination_x >= i32::from(canvas.width)
                || destination_y >= i32::from(canvas.height)
            {
                continue;
            }
            let [red, green, blue, alpha] = raster[(source_x, source_y)].to_srgba_unmultiplied();
            if alpha == 0 {
                continue;
            }
            let destination_index = usize::try_from(destination_y)
                .unwrap_or_default()
                .saturating_mul(usize::from(canvas.width))
                .saturating_add(usize::try_from(destination_x).unwrap_or_default());
            let [destination_red, destination_green, destination_blue, destination_alpha] =
                palette[usize::from(canvas.pixels[destination_index])].as_rgba();
            let source_alpha = f32::from(alpha) / 255.0;
            let destination_alpha = f32::from(destination_alpha) / 255.0;
            let output_alpha = source_alpha + destination_alpha * (1.0 - source_alpha);
            if output_alpha <= 0.0 {
                continue;
            }
            let channels = [red, green, blue];
            let destination_channels = [destination_red, destination_green, destination_blue];
            let output_channels = std::array::from_fn::<_, 3, _>(|channel| {
                ((f32::from(channels[channel]) * source_alpha
                    + f32::from(destination_channels[channel])
                        * destination_alpha
                        * (1.0 - source_alpha))
                    / output_alpha)
                    .round() as u8
            });
            let alpha = (output_alpha * 255.0).round() as u8;
            let key = (output_channels, alpha);
            let nearest = *colour_cache.entry(key).or_insert_with(|| {
                palette
                    .iter()
                    .enumerate()
                    .min_by_key(|(_, colour)| {
                        let [r, g, b, a] = colour.as_rgba();
                        let dr = i32::from(r) - i32::from(output_channels[0]);
                        let dg = i32::from(g) - i32::from(output_channels[1]);
                        let db = i32::from(b) - i32::from(output_channels[2]);
                        let da = i32::from(a) - i32::from(alpha);
                        dr * dr + dg * dg + db * db + da * da
                    })
                    .map(|(index, _)| index as u8)
                    .unwrap_or(0)
            });
            canvas.pixels[destination_index] = nearest;
        }
    }
}

fn copy_to_picture(
    canvas: &GraphicsContextCanvas,
    picture: &mut ag_iso_stack::object_pool::object::PictureGraphic,
    viewport: bool,
    max_pixels: usize,
) -> Result<(), String> {
    use ag_iso_stack::object_pool::object_attributes::{DataCodeType, PictureGraphicFormat};

    let (width, height) = (
        usize::from(picture.actual_width),
        usize::from(picture.actual_height),
    );
    let count = width
        .checked_mul(height)
        .filter(|count| *count <= max_pixels)
        .ok_or_else(|| format!("Picture Graphic exceeds the {max_pixels} pixel preview limit"))?;
    if width == 0 || height == 0 {
        return Err("Picture Graphic dimensions must be nonzero".into());
    }
    let row_bytes = match picture.format {
        PictureGraphicFormat::Monochrome => width.div_ceil(8),
        PictureGraphicFormat::FourBit => width.div_ceil(2),
        PictureGraphicFormat::EightBit => width,
    };
    let mut raw = picture.data_as_raw_encoded();
    let expected = row_bytes
        .checked_mul(height)
        .ok_or_else(|| "Picture Graphic data size overflow".to_owned())?;
    if raw.len() < expected {
        return Err("Picture Graphic data is shorter than its dimensions".into());
    }
    raw.truncate(expected);
    let mut indices = Vec::new();
    indices
        .try_reserve_exact(count)
        .map_err(|_| "unable to allocate Picture Graphic preview data".to_owned())?;
    for y in 0..height {
        for x in 0..width {
            let byte = raw[y * row_bytes
                + match picture.format {
                    PictureGraphicFormat::Monochrome => x / 8,
                    PictureGraphicFormat::FourBit => x / 2,
                    PictureGraphicFormat::EightBit => x,
                }];
            let value = match picture.format {
                PictureGraphicFormat::Monochrome => (byte >> (7 - x % 8)) & 1,
                PictureGraphicFormat::FourBit => {
                    if x % 2 == 0 {
                        byte >> 4
                    } else {
                        byte & 0x0f
                    }
                }
                PictureGraphicFormat::EightBit => byte,
            };
            indices.push(value);
        }
    }
    let allowed_colours = match picture.format {
        PictureGraphicFormat::Monochrome => 2,
        PictureGraphicFormat::FourBit => 16,
        PictureGraphicFormat::EightBit => 256,
    };
    let copy_width = width.min(usize::from(if viewport {
        canvas.viewport.2
    } else {
        canvas.width
    }));
    let copy_height = height.min(usize::from(if viewport {
        canvas.viewport.3
    } else {
        canvas.height
    }));
    for y in 0..copy_height {
        for x in 0..copy_width {
            let source_x = if viewport {
                i32::from(canvas.viewport.0) + (x as f32 / canvas.zoom) as i32
            } else {
                x as i32
            };
            let source_y = if viewport {
                i32::from(canvas.viewport.1) + (y as f32 / canvas.zoom) as i32
            } else {
                y as i32
            };
            if source_x < 0
                || source_y < 0
                || source_x >= i32::from(canvas.width)
                || source_y >= i32::from(canvas.height)
            {
                continue;
            }
            let source =
                canvas.pixels[source_y as usize * usize::from(canvas.width) + source_x as usize];
            if source == canvas.transparency_colour || usize::from(source) >= allowed_colours {
                continue;
            }
            indices[y * width + x] = source;
        }
    }
    let mut encoded = vec![0; expected];
    for y in 0..height {
        for x in 0..width {
            let value = indices[y * width + x];
            match picture.format {
                PictureGraphicFormat::Monochrome => {
                    encoded[y * row_bytes + x / 8] |= value << (7 - x % 8);
                }
                PictureGraphicFormat::FourBit => {
                    let byte = &mut encoded[y * row_bytes + x / 2];
                    if x % 2 == 0 {
                        *byte |= value << 4;
                    } else {
                        *byte |= value & 0x0f;
                    }
                }
                PictureGraphicFormat::EightBit => encoded[y * row_bytes + x] = value,
            }
        }
    }
    picture.data = encoded;
    picture.options.data_code_type = DataCodeType::Raw;
    Ok(())
}

fn nullable_id(value: Option<&String>) -> Option<Option<u16>> {
    let value = value?;
    Some(if value == "null" {
        None
    } else {
        value.parse::<u16>().ok()
    })
}

fn animation_child_at(animation: &Animation, index: u8) -> Option<ObjectRef> {
    if index == u8::MAX || usize::from(index) >= animation.object_refs.len() {
        return None;
    }
    if animation.enabled
        && !(animation.first_child_index <= animation.last_child_index
            && animation.last_child_index < animation.object_refs.len() as u8
            && usize::from(animation.default_child_index) < animation.object_refs.len())
    {
        return None;
    }
    animation.object_refs.get(usize::from(index)).cloned()
}

/// Resolve the selected frame without advancing an animation clock. Used by
/// the static renderer outside a Preview session.
pub(crate) fn animation_static_child(animation: &Animation) -> Option<ObjectRef> {
    let index = if animation.enabled {
        animation.value
    } else {
        match animation.options.disabled_behaviour {
            DisabledBehaviour::Pause => animation.value,
            DisabledBehaviour::ResetToFirst => animation.first_child_index,
            DisabledBehaviour::DefaultObject => animation.default_child_index,
            DisabledBehaviour::Blank => return None,
        }
    };
    animation_child_at(animation, index)
}

fn advance_animation_index(animation: &Animation, current: u8, steps: u128) -> u8 {
    let first = u128::from(animation.first_child_index);
    let last = u128::from(animation.last_child_index);
    if first > last || last >= animation.object_refs.len() as u128 {
        return current;
    }
    if current > animation.last_child_index {
        return match animation.options.animation_sequence {
            AnimationSequence::SingleShot => animation.last_child_index,
            AnimationSequence::Loop => (first + steps.saturating_sub(1) % (last - first + 1)) as u8,
        };
    }
    if u128::from(current) < first {
        return match animation.options.animation_sequence {
            AnimationSequence::SingleShot => (first + steps).min(last) as u8,
            AnimationSequence::Loop => (first + steps % (last - first + 1)) as u8,
        };
    }
    match animation.options.animation_sequence {
        AnimationSequence::SingleShot => (u128::from(current) + steps).min(last) as u8,
        AnimationSequence::Loop => {
            (first + (u128::from(current) - first + steps) % (last - first + 1)) as u8
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ag_iso_stack::object_pool::{
        object::{Animation, Macro, NumberVariable, Object},
        object_attributes::{AnimationOptions, MacroRef, Point},
        vt_version::VtVersion,
        ObjectType,
    };

    #[test]
    fn reset_discards_runtime_state_and_preserves_authored_pool() {
        let pool = ObjectPool::new();
        let mut session = PreviewSession::new(&pool);
        session.hidden_objects.insert(7);
        session.reset(&pool);
        assert!(!session.is_hidden(7));
        assert_eq!(session.pool(), &pool);
    }

    #[test]
    fn animation_is_shared_and_suspends_while_not_visible() {
        let mut pool = ObjectPool::new();
        let animation = Animation {
            id: ObjectId::new(10).unwrap(),
            width: 1,
            height: 1,
            refresh_interval: 100,
            value: 0,
            enabled: true,
            first_child_index: 0,
            last_child_index: 1,
            default_child_index: 0,
            options: AnimationOptions {
                animation_sequence: AnimationSequence::Loop,
                disabled_behaviour: DisabledBehaviour::Pause,
            },
            object_refs: vec![0, 1]
                .into_iter()
                .map(|i| ObjectRef {
                    id: ObjectId::new(i).unwrap(),
                    offset: Point { x: 0, y: 0 },
                })
                .collect(),
            macro_refs: vec![],
        };
        pool.add(Object::Animation(animation));
        let mut session = PreviewSession::new(&pool);
        session.begin_frame(Duration::ZERO);
        let _ = session.animation_child(10);
        assert_eq!(session.animation_child(10).unwrap().id.value(), 0);
        session.finish_frame();
        session.begin_frame(Duration::from_millis(50));
        assert_eq!(session.animation_child(10).unwrap().id.value(), 0);
        session.finish_frame();
        session.begin_frame(Duration::from_millis(100));
        assert_eq!(session.animation_child(10).unwrap().id.value(), 1);
        session.finish_frame();
        session.begin_frame(Duration::from_millis(10_000));
        session.finish_frame();
        session.begin_frame(Duration::from_millis(10_100));
        assert_eq!(session.animation_child(10).unwrap().id.value(), 1);
        session.finish_frame();
        session.begin_frame(Duration::from_millis(10_200));
        assert_eq!(session.animation_child(10).unwrap().id.value(), 0);
        session.finish_frame();
        session.set_object_enabled(10, false).unwrap();
        session.set_object_enabled(10, true).unwrap();
        let Some(Object::Animation(animation)) =
            session.pool().object_by_id(ObjectId::new(10).unwrap())
        else {
            unreachable!();
        };
        assert_eq!(animation.value, 0);
    }

    fn macro_object(id: u16, name: &str, arguments: &[&str]) -> Object {
        let bytes = crate::macro_authoring::encode_macro_instruction(
            name,
            &arguments
                .iter()
                .map(|value| (*value).to_owned())
                .collect::<Vec<_>>(),
            VtVersion::Version6,
        )
        .unwrap();
        Object::Macro(Macro {
            id: ObjectId::new(id).unwrap(),
            commands: bytes,
        })
    }

    fn output_number(id: u16, macros: &[(u16, Event)]) -> Object {
        let Object::OutputNumber(mut object) =
            crate::object_defaults::default_object(ObjectType::OutputNumber)
        else {
            unreachable!();
        };
        object.id = ObjectId::new(id).unwrap();
        object.macro_refs = macros
            .iter()
            .map(|(macro_id, event_id)| MacroRef {
                macro_id: *macro_id,
                event_id: *event_id,
            })
            .collect();
        Object::OutputNumber(object)
    }

    #[test]
    fn macro_changes_are_temporary_and_reset_restores_authored_values() {
        let mut pool = ObjectPool::new();
        pool.add(Object::NumberVariable(NumberVariable {
            id: ObjectId::new(10).unwrap(),
            value: 5,
        }));
        pool.add(macro_object(20, "change-numeric-value", &["10", "42"]));
        let mut session = PreviewSession::new(&pool);
        session.execute_macro(20).unwrap();
        let Object::NumberVariable(variable) = session
            .pool()
            .object_by_id(ObjectId::new(10).unwrap())
            .unwrap()
        else {
            unreachable!();
        };
        assert_eq!(variable.value, 42);
        session.reset(&pool);
        let Object::NumberVariable(variable) = session
            .pool()
            .object_by_id(ObjectId::new(10).unwrap())
            .unwrap()
        else {
            unreachable!();
        };
        assert_eq!(variable.value, 5);
        let Object::NumberVariable(authored) =
            pool.object_by_id(ObjectId::new(10).unwrap()).unwrap()
        else {
            unreachable!();
        };
        assert_eq!(authored.value, 5);
    }

    #[test]
    fn event_macros_keep_sibling_before_nested_fifo_order() {
        let mut pool = ObjectPool::new();
        pool.add(output_number(
            1,
            &[(20, Event::OnChangeValue), (21, Event::OnChangeValue)],
        ));
        pool.add(output_number(2, &[(22, Event::OnChangeValue)]));
        pool.add(Object::NumberVariable(NumberVariable {
            id: ObjectId::new(3).unwrap(),
            value: 0,
        }));
        pool.add(macro_object(20, "change-numeric-value", &["2", "1"]));
        pool.add(macro_object(21, "change-numeric-value", &["3", "2"]));
        pool.add(macro_object(22, "change-numeric-value", &["3", "3"]));
        let mut session = PreviewSession::new(&pool);
        session.trigger_event(1, Event::OnChangeValue).unwrap();
        let Object::NumberVariable(variable) = session
            .pool()
            .object_by_id(ObjectId::new(3).unwrap())
            .unwrap()
        else {
            unreachable!();
        };
        assert_eq!(variable.value, 3, "queued C must run after sibling B");
    }

    #[test]
    fn recursive_event_macro_cycle_is_bounded_and_reported() {
        let mut pool = ObjectPool::new();
        pool.add(output_number(1, &[(20, Event::OnChangeValue)]));
        pool.add(macro_object(20, "change-numeric-value", &["1", "1"]));
        let mut session = PreviewSession::new(&pool);
        assert!(session.trigger_event(1, Event::OnChangeValue).is_err());
        assert!(session
            .diagnostics()
            .iter()
            .any(|diagnostic| diagnostic.message.contains("4096")));
    }

    #[test]
    fn graphics_context_background_command_preserves_canvas_and_reset_discards_it() {
        let mut pool = ObjectPool::new();
        let Object::GraphicsContext(mut graphics) =
            crate::object_defaults::default_object(ObjectType::GraphicsContext)
        else {
            unreachable!();
        };
        graphics.id = ObjectId::new(5).unwrap();
        graphics.canvas_width = 2;
        graphics.canvas_height = 1;
        graphics.viewport_width = 2;
        graphics.viewport_height = 1;
        graphics.viewport_zoom = 1.0;
        graphics.background_colour = 4;
        pool.add(Object::GraphicsContext(graphics));
        let mut session = PreviewSession::new(&pool);
        for instruction in [
            MacroInstruction {
                name: "graphics-context".into(),
                arguments: vec!["5".into(), "set-background-colour".into(), "7".into()],
            },
            MacroInstruction {
                name: "graphics-context".into(),
                arguments: vec!["5".into(), "draw-point".into(), "0".into(), "0".into()],
            },
        ] {
            session.execute_instruction(1, &instruction).unwrap();
        }
        let canvas = session.graphics_context(5).unwrap();
        assert_eq!(canvas.pixels, vec![0, 4]);
        assert_eq!(canvas.background, 7);
        session.reset(&pool);
        let canvas = session.graphics_context(5).unwrap();
        assert_eq!(canvas.pixels, vec![4, 4]);
    }

    #[test]
    fn graphics_context_uses_shape_attributes_and_text_extent() {
        let mut pool = ObjectPool::new();
        let Object::GraphicsContext(mut graphics) =
            crate::default_object(ObjectType::GraphicsContext)
        else {
            unreachable!();
        };
        graphics.id = ObjectId::new(5).unwrap();
        graphics.canvas_width = 32;
        graphics.canvas_height = 16;
        graphics.viewport_width = 32;
        graphics.viewport_height = 16;
        graphics.viewport_zoom = 1.0;
        graphics.background_colour = 0;
        graphics.foreground_colour = 12;
        graphics.line_attributes_object = ObjectId::new(6).unwrap().into();
        graphics.font_attributes_object = ObjectId::new(7).unwrap().into();
        pool.add(Object::GraphicsContext(graphics));
        let Object::LineAttributes(mut line) = crate::default_object(ObjectType::LineAttributes)
        else {
            unreachable!();
        };
        line.id = ObjectId::new(6).unwrap();
        line.line_width = 2;
        line.line_art = 65535;
        line.line_colour = 12;
        pool.add(Object::LineAttributes(line));
        let mut font = crate::default_object(ObjectType::FontAttributes);
        *font.mut_id() = ObjectId::new(7).unwrap();
        pool.add(font);
        let mut session = PreviewSession::new(&pool);
        let command = |subcommand: &str, values: &[&str]| MacroInstruction {
            name: "graphics-context".into(),
            arguments: ["5", subcommand]
                .into_iter()
                .chain(values.iter().copied())
                .map(str::to_owned)
                .collect(),
        };
        session
            .execute_instruction(1, &command("draw-rectangle", &["8", "8"]))
            .unwrap();
        let canvas = session.graphics_context(5).unwrap();
        assert_eq!(canvas.pixels[3], 12);
        assert_eq!(
            canvas.pixels[32 + 3],
            12,
            "the second border row must be painted"
        );
        assert_eq!(
            canvas.pixels[3 * 32 + 3],
            0,
            "NULL fill leaves the interior unchanged"
        );
        session
            .execute_instruction(1, &command("set-cursor", &["0", "0"]))
            .unwrap();
        session
            .execute_instruction(1, &command("draw-text", &["transparent", "A"]))
            .unwrap();
        assert_eq!(session.graphics_context(5).unwrap().cursor, (5, 7));
    }
}
