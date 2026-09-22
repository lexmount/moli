use std::{
    borrow::Cow,
    collections::{BTreeMap, BTreeSet},
};

use moli_protocol::devtools_runtime::{
    DevToolsCallFunctionCommand, DevToolsCommand, DevToolsDispatchKeyEventCommand,
    DevToolsDispatchMouseEventCommand, DevToolsDispatchTouchEventCommand,
    DevToolsDomGeometryResult, DevToolsKeyEventType, DevToolsMouseEventType, DevToolsPointerType,
    DevToolsRemoteHandleId, DevToolsResultOwnership, DevToolsTouchEventType, DevToolsTouchPoint,
};
use serde_json::Value;

use crate::{
    CLASSIC_ELEMENT_REFERENCE_KEY, ClassicDevToolsCommandContext, ClassicError, ClassicErrorCode,
    cdp_node_id_from_classic_element_id, geometry_border_quad, required_object_string,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClassicViewportPoint {
    pub x: f64,
    pub y: f64,
}

impl ClassicViewportPoint {
    pub fn new(x: f64, y: f64) -> Result<Self, ClassicError> {
        if !x.is_finite() || !y.is_finite() {
            return Err(ClassicError::new(
                ClassicErrorCode::UnknownError,
                "element origin has invalid viewport coordinates",
            ));
        }
        Ok(Self { x, y })
    }

    fn offset(self, x: f64, y: f64) -> Result<Self, ClassicError> {
        Self::new(self.x + x, self.y + y)
    }
}

pub type ClassicElementOriginViewportPoints = BTreeMap<String, ClassicViewportPoint>;

#[derive(Debug, Clone, PartialEq)]
pub struct ClassicActionTick {
    pub commands: Vec<DevToolsCommand>,
    pub duration_ms: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ClassicViewportBounds {
    pub width: f64,
    pub height: f64,
}

impl ClassicViewportBounds {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width: f64::from(width),
            height: f64::from(height),
        }
    }

    fn contains(self, point: ClassicViewportPoint) -> bool {
        point.x >= 0.0 && point.x <= self.width && point.y >= 0.0 && point.y <= self.height
    }
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ClassicActionState {
    key_sources: BTreeMap<String, ClassicKeyInputState>,
    pointer_sources: BTreeMap<String, ClassicPointerInputState>,
    wheel_sources: BTreeSet<String>,
    none_sources: BTreeSet<String>,
    cancel_list: Vec<ClassicActionCancel>,
}

#[derive(Debug, Clone, Default, PartialEq)]
struct ClassicKeyInputState {
    pressed: BTreeMap<String, ClassicWebDriverKey>,
    modifiers: u8,
}

#[derive(Debug, Clone, PartialEq)]
struct ClassicPointerInputState {
    pointer_type: String,
    x: f64,
    y: f64,
    buttons: i32,
    touch_identifier: Option<i32>,
    click_count: i32,
    last_click: Option<ClassicPointerClick>,
}

impl ClassicPointerInputState {
    fn new(pointer_type: impl Into<String>) -> Self {
        Self {
            pointer_type: pointer_type.into(),
            x: 0.0,
            y: 0.0,
            buttons: 0,
            touch_identifier: None,
            click_count: 0,
            last_click: None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ClassicPointerClick {
    x: f64,
    y: f64,
    button: i32,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct ClassicPointerEventProperties {
    pressure: f64,
    tangential_pressure: f64,
    tilt_x: f64,
    tilt_y: f64,
    twist: f64,
}

impl ClassicPointerEventProperties {
    fn mouse_default() -> Self {
        Self {
            pressure: 0.0,
            tangential_pressure: 0.0,
            tilt_x: 0.0,
            tilt_y: 0.0,
            twist: 0.0,
        }
    }

    fn pointer_action_default() -> Self {
        Self {
            pressure: 0.5,
            tangential_pressure: 0.0,
            tilt_x: 0.0,
            tilt_y: 0.0,
            twist: 0.0,
        }
    }

    fn with_pressure(self, pressure: f64) -> Self {
        Self { pressure, ..self }
    }
}

#[derive(Debug, Clone, PartialEq)]
enum ClassicActionCancel {
    Key {
        source_id: String,
        key: ClassicWebDriverKey,
    },
    Pointer {
        source_id: String,
        button: i32,
    },
    Touch {
        source_id: String,
    },
}

pub fn element_click_input_commands(
    context: &ClassicDevToolsCommandContext,
    geometry: &DevToolsDomGeometryResult,
) -> Result<Vec<DevToolsCommand>, ClassicError> {
    let point = element_center_from_geometry(geometry)?;
    Ok(vec![
        dispatch_mouse_event_command(
            context,
            DevToolsMouseEventType::Pressed,
            DevToolsPointerType::Mouse,
            point.x,
            point.y,
            0,
            Some(1),
        ),
        dispatch_mouse_event_command(
            context,
            DevToolsMouseEventType::Released,
            DevToolsPointerType::Mouse,
            point.x,
            point.y,
            0,
            Some(0),
        ),
    ])
}

pub fn element_send_keys_text(params: &Value) -> Result<String, ClassicError> {
    if let Some(text) = params.get("text") {
        return text.as_str().map(ToOwned::to_owned).ok_or_else(|| {
            ClassicError::new(ClassicErrorCode::InvalidArgument, "text must be a string")
        });
    }
    let value = params
        .get("value")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "text must be a string or value must be an array",
            )
        })?;
    let mut text = String::new();
    for item in value {
        let Some(item) = item.as_str() else {
            return Err(ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "value entries must be strings",
            ));
        };
        text.push_str(item);
    }
    Ok(text)
}

pub fn element_send_keys_input_commands(
    context: &ClassicDevToolsCommandContext,
    text: &str,
) -> Vec<DevToolsCommand> {
    let mut commands = Vec::new();
    let mut pressed_modifiers: Vec<ClassicWebDriverKey> = Vec::new();
    let mut pressed_modifier_keys: BTreeSet<String> = BTreeSet::new();
    let mut modifiers = 0u8;
    for character in text.chars() {
        if character == '\u{E000}' {
            element_send_keys_release_modifiers(
                context,
                &mut commands,
                &mut pressed_modifiers,
                &mut pressed_modifier_keys,
                &mut modifiers,
            );
            continue;
        }
        let key = webdriver_key_for_action_character(character);
        if let Some(mask) = key.modifier_mask {
            let auto_repeat = pressed_modifier_keys.contains(&key.key);
            modifiers |= mask;
            if !auto_repeat {
                pressed_modifier_keys.insert(key.key.clone());
                pressed_modifiers.push(key.clone());
            }
            let event_key = key.event_key(modifiers);
            commands.push(dispatch_key_event_command_with_modifiers(
                context,
                DevToolsKeyEventType::KeyDown,
                event_key.as_ref(),
                &key.code,
                "",
                modifiers,
                auto_repeat,
                false,
            ));
            continue;
        }

        let event_key = key.event_key(modifiers);
        let input_text = key.input_text(modifiers);
        commands.push(dispatch_key_event_command_with_modifiers(
            context,
            DevToolsKeyEventType::KeyDown,
            event_key.as_ref(),
            &key.code,
            input_text.as_ref(),
            modifiers,
            false,
            key.should_insert_text(modifiers),
        ));
        commands.push(dispatch_key_event_command_with_modifiers(
            context,
            DevToolsKeyEventType::KeyUp,
            event_key.as_ref(),
            &key.code,
            "",
            modifiers,
            false,
            false,
        ));
    }
    element_send_keys_release_modifiers(
        context,
        &mut commands,
        &mut pressed_modifiers,
        &mut pressed_modifier_keys,
        &mut modifiers,
    );
    commands
}

fn element_send_keys_release_modifiers(
    context: &ClassicDevToolsCommandContext,
    commands: &mut Vec<DevToolsCommand>,
    pressed_modifiers: &mut Vec<ClassicWebDriverKey>,
    pressed_modifier_keys: &mut BTreeSet<String>,
    modifiers: &mut u8,
) {
    for key in pressed_modifiers.drain(..).rev() {
        pressed_modifier_keys.remove(&key.key);
        if let Some(mask) = key.modifier_mask {
            *modifiers &= !mask;
        }
        let event_key = key.event_key(*modifiers);
        commands.push(dispatch_key_event_command_with_modifiers(
            context,
            DevToolsKeyEventType::KeyUp,
            event_key.as_ref(),
            &key.code,
            "",
            *modifiers,
            false,
            false,
        ));
    }
}

pub fn element_send_keys_prepare_text_control_command(
    context: &ClassicDevToolsCommandContext,
    object_id: impl Into<String>,
    text: impl Into<String>,
) -> DevToolsCommand {
    DevToolsCommand::CallFunction(DevToolsCallFunctionCommand {
        context: context.command_context(),
        realm_id: None,
        world_name: None,
        object_id: Some(DevToolsRemoteHandleId::from(object_id.into())),
        this_parameter: None,
        function_declaration: r#"function() {
            if (!this || this.nodeType !== Node.ELEMENT_NODE || !this.isConnected) {
                throw new Error('__moli_webdriver_classic_stale_element_reference__');
            }
            const localName = String(this.localName || '').toLowerCase();
            const type = localName === 'input' ? String(this.type || '').toLowerCase() : '';
            const isTextControl =
                localName === 'textarea' ||
                (localName === 'input' && ['text', 'search', 'tel', 'url', 'password'].includes(type));
            const isWholeValueInput =
                localName === 'input' &&
                ['color', 'date', 'datetime-local', 'month', 'number', 'range', 'time', 'week'].includes(type);
            if (isWholeValueInput) {
                this.focus();
                this.value = arguments[0];
                this.dispatchEvent(new Event('input', { bubbles: true }));
                this.dispatchEvent(new Event('change', { bubbles: true }));
                return 'value-set';
            }
            if (!isTextControl) {
                return 'not-text-control';
            }

            const doc = this.ownerDocument || document;
            const active = doc.activeElement;
            const wasFocused = active === this;
            if (!wasFocused) {
                if (active && typeof active.blur === 'function') {
                    active.blur();
                }
                this.focus();
                if (doc.activeElement !== this) {
                    throw new Error('__moli_webdriver_classic_element_not_interactable__');
                }
                this.setSelectionRange(this.value.length, this.value.length);
            }
            return 'text-control';
        }"#
        .to_owned(),
        arguments: vec![serde_json::json!(text.into())],
        await_promise: false,
        user_gesture: false,
        webdriver_bidi_file_prompt_handler: None,
        result_ownership: DevToolsResultOwnership::None,
        object_group: None,
        preserve_remote_metadata: false,
        materialize_bidi_script_result: false,
        serialization_options: None,
    })
}

pub fn perform_actions_commands(
    context: &ClassicDevToolsCommandContext,
    params: &Value,
) -> Result<Vec<DevToolsCommand>, ClassicError> {
    perform_actions_commands_with_element_origins(
        context,
        params,
        &ClassicElementOriginViewportPoints::new(),
    )
}

pub fn perform_actions_commands_with_element_origins(
    context: &ClassicDevToolsCommandContext,
    params: &Value,
    element_origins: &ClassicElementOriginViewportPoints,
) -> Result<Vec<DevToolsCommand>, ClassicError> {
    let mut action_state = ClassicActionState::default();
    perform_actions_commands_with_state(context, params, element_origins, &mut action_state)
}

pub fn perform_actions_commands_with_state(
    context: &ClassicDevToolsCommandContext,
    params: &Value,
    element_origins: &ClassicElementOriginViewportPoints,
    action_state: &mut ClassicActionState,
) -> Result<Vec<DevToolsCommand>, ClassicError> {
    perform_actions_commands_with_state_and_viewport(
        context,
        params,
        element_origins,
        None,
        action_state,
    )
}

pub fn perform_actions_commands_with_state_and_viewport(
    context: &ClassicDevToolsCommandContext,
    params: &Value,
    element_origins: &ClassicElementOriginViewportPoints,
    viewport_bounds: Option<ClassicViewportBounds>,
    action_state: &mut ClassicActionState,
) -> Result<Vec<DevToolsCommand>, ClassicError> {
    Ok(perform_actions_ticks_with_state_and_viewport(
        context,
        params,
        element_origins,
        viewport_bounds,
        action_state,
    )?
    .into_iter()
    .flat_map(|tick| tick.commands)
    .collect())
}

pub fn perform_actions_ticks_with_state_and_viewport(
    context: &ClassicDevToolsCommandContext,
    params: &Value,
    element_origins: &ClassicElementOriginViewportPoints,
    viewport_bounds: Option<ClassicViewportBounds>,
    action_state: &mut ClassicActionState,
) -> Result<Vec<ClassicActionTick>, ClassicError> {
    let actions = params
        .get("actions")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "actions must be an array",
            )
        })?;
    let mut source_ids = BTreeSet::new();
    let mut sources = Vec::new();
    for action in actions {
        let source = ClassicActionSource::parse(action, action_state)?;
        if !source_ids.insert(source.id().to_owned()) {
            return Err(ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "action source id must be unique",
            ));
        }
        sources.push(source);
    }
    assign_touch_identifiers(&mut sources, action_state);
    let max_ticks = sources
        .iter()
        .map(ClassicActionSource::action_count)
        .max()
        .unwrap_or(0);
    let mut ticks = Vec::new();
    let mut cancel_list = Vec::new();

    for tick in 0..max_ticks {
        let mut commands = Vec::new();
        let mut duration_ms = 0;
        for source in &mut sources {
            let Some(action) = source.action_at(tick) else {
                continue;
            };
            duration_ms = duration_ms.max(source.action_duration_ms(&action)?);
            source.append_action_command(
                context,
                &action,
                &mut commands,
                element_origins,
                viewport_bounds,
                &mut cancel_list,
            )?;
        }
        ticks.push(ClassicActionTick {
            commands: coalesce_tick_touch_dispatch_commands(commands),
            duration_ms,
        });
    }

    action_state.cancel_list.extend(cancel_list);
    for source in sources {
        source.commit_state(action_state);
    }

    Ok(ticks)
}

fn assign_touch_identifiers(
    sources: &mut [ClassicActionSource],
    action_state: &ClassicActionState,
) {
    let mut used = action_state
        .pointer_sources
        .values()
        .filter_map(|source| source.touch_identifier)
        .collect::<BTreeSet<_>>();
    let mut next = used.iter().next_back().map(|id| id + 1).unwrap_or(0);
    for source in sources {
        let ClassicActionSource::Pointer(pointer) = source else {
            continue;
        };
        if pointer.pointer_type != "touch" || pointer.touch_identifier.is_some() {
            continue;
        }
        while used.contains(&next) {
            next += 1;
        }
        pointer.touch_identifier = Some(next);
        used.insert(next);
        next += 1;
    }
}

fn coalesce_tick_touch_dispatch_commands(commands: Vec<DevToolsCommand>) -> Vec<DevToolsCommand> {
    let Some(last_touch_index) = commands
        .iter()
        .enumerate()
        .filter_map(|(index, command)| match command {
            DevToolsCommand::DispatchTouchEvent(_) => Some(index),
            _ => None,
        })
        .next_back()
    else {
        return commands;
    };

    let mut touch_commands: Vec<DevToolsDispatchTouchEventCommand> = Vec::new();
    for command in &commands {
        let DevToolsCommand::DispatchTouchEvent(command) = command else {
            continue;
        };
        match touch_commands.last_mut() {
            Some(previous)
                if previous.context == command.context
                    && previous.event_type == command.event_type =>
            {
                previous
                    .touch_points
                    .extend(command.touch_points.iter().copied());
            }
            _ => touch_commands.push(command.clone()),
        }
    }

    let mut coalesced = Vec::new();
    for (index, command) in commands.into_iter().enumerate() {
        if index == last_touch_index {
            coalesced.extend(
                touch_commands
                    .iter()
                    .cloned()
                    .map(DevToolsCommand::DispatchTouchEvent),
            );
        }
        if !matches!(command, DevToolsCommand::DispatchTouchEvent(_)) {
            coalesced.push(command);
        }
    }
    coalesced
}

pub fn release_actions_commands(
    context: &ClassicDevToolsCommandContext,
    action_state: &mut ClassicActionState,
) -> Vec<DevToolsCommand> {
    let mut commands = Vec::new();
    let cancel_list = std::mem::take(&mut action_state.cancel_list);
    for cancel in cancel_list.into_iter().rev() {
        match cancel {
            ClassicActionCancel::Key { source_id, key } => {
                let Some(source) = action_state.key_sources.get_mut(&source_id) else {
                    continue;
                };
                if source.pressed.remove(&key.key).is_none() {
                    continue;
                }
                if let Some(mask) = key.modifier_mask {
                    source.modifiers &= !mask;
                }
                let event_key = key.event_key(source.modifiers);
                commands.push(dispatch_key_event_command_with_modifiers(
                    context,
                    DevToolsKeyEventType::KeyUp,
                    event_key.as_ref(),
                    &key.code,
                    "",
                    source.modifiers,
                    false,
                    false,
                ));
            }
            ClassicActionCancel::Pointer { source_id, button } => {
                let Some(source) = action_state.pointer_sources.get_mut(&source_id) else {
                    continue;
                };
                if source.pointer_type == "touch" {
                    continue;
                }
                let mask = pointer_button_mask(button);
                if source.buttons & mask == 0 {
                    continue;
                }
                source.buttons &= !mask;
                commands.push(dispatch_mouse_event_command(
                    context,
                    DevToolsMouseEventType::Released,
                    pointer_type_for_mouse_source(&source.pointer_type),
                    source.x,
                    source.y,
                    button,
                    Some(source.buttons),
                ));
            }
            ClassicActionCancel::Touch { source_id } => {
                let Some(source) = action_state.pointer_sources.get_mut(&source_id) else {
                    continue;
                };
                if source.pointer_type != "touch" || source.buttons == 0 {
                    continue;
                }
                source.buttons = 0;
                commands.push(dispatch_touch_event_command(
                    context,
                    DevToolsTouchEventType::End,
                    source.touch_identifier.unwrap_or(0),
                    source.x,
                    source.y,
                ));
            }
        }
    }
    *action_state = ClassicActionState::default();
    coalesce_tick_touch_dispatch_commands(commands)
}

pub fn action_element_origin_ids(params: &Value) -> Result<Vec<String>, ClassicError> {
    let actions = params
        .get("actions")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "actions must be an array",
            )
        })?;
    let mut element_ids = BTreeSet::new();
    for source in actions {
        let object = source.as_object().ok_or_else(|| {
            ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "action source must be an object",
            )
        })?;
        let source_type = required_object_string(object, "type")?;
        if !matches!(source_type, "pointer" | "wheel") {
            continue;
        }
        for action in action_source_actions(object)? {
            let object = action.as_object().ok_or_else(|| {
                ClassicError::new(
                    ClassicErrorCode::InvalidArgument,
                    format!("{source_type} action must be an object"),
                )
            })?;
            let action_type = required_object_string(object, "type")?;
            if !matches!(
                (source_type, action_type),
                ("pointer", "pointerMove") | ("wheel", "scroll")
            ) {
                continue;
            }
            if let Some(Value::Object(origin)) = object.get("origin") {
                element_ids.insert(element_origin_reference(origin)?);
            }
        }
    }
    Ok(element_ids.into_iter().collect())
}

enum ClassicActionSource {
    Pointer(ClassicPointerActionSource),
    Key(ClassicKeyActionSource),
    Wheel(ClassicWheelActionSource),
    None(ClassicNoneActionSource),
}

impl ClassicActionSource {
    fn parse(value: &Value, action_state: &ClassicActionState) -> Result<Self, ClassicError> {
        let object = value.as_object().ok_or_else(|| {
            ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "action source must be an object",
            )
        })?;
        match required_object_string(object, "type")? {
            "pointer" => ClassicPointerActionSource::parse(object, action_state).map(Self::Pointer),
            "key" => ClassicKeyActionSource::parse(object, action_state).map(Self::Key),
            "wheel" => ClassicWheelActionSource::parse(object, action_state).map(Self::Wheel),
            "none" => ClassicNoneActionSource::parse(object, action_state).map(Self::None),
            _ => Err(ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "only none, pointer, key, and wheel action sources are supported",
            )),
        }
    }

    fn id(&self) -> &str {
        match self {
            Self::Pointer(source) => &source.id,
            Self::Key(source) => &source.id,
            Self::Wheel(source) => &source.id,
            Self::None(source) => &source.id,
        }
    }

    fn action_count(&self) -> usize {
        match self {
            Self::Pointer(source) => source.actions.len(),
            Self::Key(source) => source.actions.len(),
            Self::Wheel(source) => source.actions.len(),
            Self::None(source) => source.actions.len(),
        }
    }

    fn action_at(&self, tick: usize) -> Option<Value> {
        match self {
            Self::Pointer(source) => source.actions.get(tick).cloned(),
            Self::Key(source) => source.actions.get(tick).cloned(),
            Self::Wheel(source) => source.actions.get(tick).cloned(),
            Self::None(source) => source.actions.get(tick).cloned(),
        }
    }

    fn action_duration_ms(&self, action: &Value) -> Result<u64, ClassicError> {
        let object = action.as_object().ok_or_else(|| {
            ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "action must be an object",
            )
        })?;
        let action_type = required_object_string(object, "type")?;
        let duration_applies = match self {
            Self::Pointer(_) => matches!(action_type, "pause" | "pointerMove"),
            Self::Key(_) => action_type == "pause",
            Self::Wheel(_) => matches!(action_type, "pause" | "scroll"),
            Self::None(_) => action_type == "pause",
        };
        if duration_applies {
            optional_duration_ms(object)
        } else {
            Ok(0)
        }
    }

    fn append_action_command(
        &mut self,
        context: &ClassicDevToolsCommandContext,
        action: &Value,
        commands: &mut Vec<DevToolsCommand>,
        element_origins: &ClassicElementOriginViewportPoints,
        viewport_bounds: Option<ClassicViewportBounds>,
        cancel_list: &mut Vec<ClassicActionCancel>,
    ) -> Result<(), ClassicError> {
        match self {
            Self::Pointer(source) => source.append_action_command(
                context,
                action,
                commands,
                element_origins,
                viewport_bounds,
                cancel_list,
            ),
            Self::Key(source) => {
                source.append_action_command(context, action, commands, cancel_list)
            }
            Self::Wheel(source) => source.append_action_command(
                context,
                action,
                commands,
                element_origins,
                viewport_bounds,
            ),
            Self::None(source) => source.append_action_command(action),
        }
    }

    fn commit_state(self, action_state: &mut ClassicActionState) {
        match self {
            Self::Pointer(source) => {
                action_state.pointer_sources.insert(
                    source.id,
                    ClassicPointerInputState {
                        pointer_type: source.pointer_type,
                        x: source.x,
                        y: source.y,
                        buttons: source.buttons,
                        touch_identifier: source.touch_identifier,
                        click_count: source.click_count,
                        last_click: source.last_click,
                    },
                );
            }
            Self::Key(source) => {
                action_state.key_sources.insert(
                    source.id,
                    ClassicKeyInputState {
                        pressed: source.pressed,
                        modifiers: source.modifiers,
                    },
                );
            }
            Self::Wheel(source) => {
                action_state.wheel_sources.insert(source.id);
            }
            Self::None(source) => {
                action_state.none_sources.insert(source.id);
            }
        }
    }
}

#[derive(Debug, Clone)]
struct ClassicNoneActionSource {
    id: String,
    actions: Vec<Value>,
}

impl ClassicNoneActionSource {
    fn parse(
        object: &serde_json::Map<String, Value>,
        action_state: &ClassicActionState,
    ) -> Result<Self, ClassicError> {
        let id = required_action_source_id(object)?;
        if action_state.pointer_sources.contains_key(id)
            || action_state.key_sources.contains_key(id)
            || action_state.wheel_sources.contains(id)
        {
            return Err(ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "action source id is already used by a different source type",
            ));
        }
        Ok(Self {
            id: id.to_owned(),
            actions: action_source_actions(object)?,
        })
    }

    fn append_action_command(&mut self, action: &Value) -> Result<(), ClassicError> {
        let object = action.as_object().ok_or_else(|| {
            ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "none action must be an object",
            )
        })?;
        match required_object_string(object, "type")? {
            "pause" => Ok(()),
            _ => Err(ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "unsupported none action type",
            )),
        }
    }
}

#[derive(Debug, Clone)]
struct ClassicPointerActionSource {
    id: String,
    pointer_type: String,
    actions: Vec<Value>,
    x: f64,
    y: f64,
    buttons: i32,
    touch_identifier: Option<i32>,
    click_count: i32,
    last_click: Option<ClassicPointerClick>,
}

impl ClassicPointerActionSource {
    fn parse(
        object: &serde_json::Map<String, Value>,
        action_state: &ClassicActionState,
    ) -> Result<Self, ClassicError> {
        let id = required_action_source_id(object)?;
        let pointer_type = object
            .get("parameters")
            .and_then(Value::as_object)
            .and_then(|parameters| parameters.get("pointerType"))
            .and_then(Value::as_str)
            .unwrap_or("mouse");
        if !matches!(pointer_type, "mouse" | "pen" | "touch") {
            return Err(ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "only mouse, pen, and touch pointer actions are supported",
            ));
        }
        if action_state.key_sources.contains_key(id)
            || action_state.wheel_sources.contains(id)
            || action_state.none_sources.contains(id)
        {
            return Err(ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "action source id is already used by a different source type",
            ));
        }
        let input_state = match action_state.pointer_sources.get(id) {
            Some(input_state) if input_state.pointer_type == pointer_type => input_state.clone(),
            Some(_) => {
                return Err(ClassicError::new(
                    ClassicErrorCode::InvalidArgument,
                    "pointerType must match the existing pointer source",
                ));
            }
            None => ClassicPointerInputState::new(pointer_type),
        };
        Ok(Self {
            id: id.to_owned(),
            pointer_type: pointer_type.to_owned(),
            actions: action_source_actions(object)?,
            x: input_state.x,
            y: input_state.y,
            buttons: input_state.buttons,
            touch_identifier: input_state.touch_identifier,
            click_count: input_state.click_count,
            last_click: input_state.last_click,
        })
    }

    fn append_action_command(
        &mut self,
        context: &ClassicDevToolsCommandContext,
        action: &Value,
        commands: &mut Vec<DevToolsCommand>,
        element_origins: &ClassicElementOriginViewportPoints,
        viewport_bounds: Option<ClassicViewportBounds>,
        cancel_list: &mut Vec<ClassicActionCancel>,
    ) -> Result<(), ClassicError> {
        let object = action.as_object().ok_or_else(|| {
            ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "pointer action must be an object",
            )
        })?;
        match required_object_string(object, "type")? {
            "pause" => Ok(()),
            "pointerMove" => {
                let point = self.pointer_move_coordinates(object, element_origins)?;
                let properties = pointer_event_properties(object)?;
                validate_action_target_in_viewport(point, viewport_bounds)?;
                self.x = point.x;
                self.y = point.y;
                if self.pointer_type == "touch" {
                    if self.buttons != 0 {
                        commands.push(dispatch_touch_event_command(
                            context,
                            DevToolsTouchEventType::Move,
                            self.touch_identifier.unwrap_or(0),
                            self.x,
                            self.y,
                        ));
                    }
                    return Ok(());
                }
                let properties = if self.buttons == 0 {
                    properties.with_pressure(0.0)
                } else {
                    properties
                };
                commands.push(dispatch_mouse_event_command_with_pointer_properties(
                    context,
                    DevToolsMouseEventType::Moved,
                    pointer_type_for_mouse_source(&self.pointer_type),
                    self.x,
                    self.y,
                    0,
                    Some(self.buttons),
                    properties,
                ));
                Ok(())
            }
            "pointerDown" => {
                let properties = pointer_event_properties(object)?;
                if self.pointer_type == "touch" {
                    if self.buttons == 0 {
                        cancel_list.push(ClassicActionCancel::Touch {
                            source_id: self.id.clone(),
                        });
                    }
                    self.buttons = 1;
                    commands.push(dispatch_touch_event_command(
                        context,
                        DevToolsTouchEventType::Start,
                        self.touch_identifier.unwrap_or(0),
                        self.x,
                        self.y,
                    ));
                    return Ok(());
                }
                let button = pointer_button_code(object)?;
                let mask = pointer_button_mask(button);
                if self.buttons & mask == 0 {
                    cancel_list.push(ClassicActionCancel::Pointer {
                        source_id: self.id.clone(),
                        button,
                    });
                }
                self.buttons |= mask;
                let click_count = self.next_click_count(button);
                commands.push(
                    dispatch_mouse_event_command_with_pointer_properties_and_click_count(
                        context,
                        DevToolsMouseEventType::Pressed,
                        pointer_type_for_mouse_source(&self.pointer_type),
                        self.x,
                        self.y,
                        button,
                        Some(self.buttons),
                        properties,
                        click_count,
                    ),
                );
                Ok(())
            }
            "pointerUp" => {
                let properties = pointer_event_properties(object)?.with_pressure(0.0);
                if self.pointer_type == "touch" {
                    if self.buttons != 0 {
                        self.buttons = 0;
                        commands.push(dispatch_touch_event_command(
                            context,
                            DevToolsTouchEventType::End,
                            self.touch_identifier.unwrap_or(0),
                            self.x,
                            self.y,
                        ));
                    }
                    return Ok(());
                }
                let button = pointer_button_code(object)?;
                self.buttons &= !pointer_button_mask(button);
                let click_count = self.record_click_release(button);
                commands.push(
                    dispatch_mouse_event_command_with_pointer_properties_and_click_count(
                        context,
                        DevToolsMouseEventType::Released,
                        pointer_type_for_mouse_source(&self.pointer_type),
                        self.x,
                        self.y,
                        button,
                        Some(self.buttons),
                        properties,
                        click_count,
                    ),
                );
                Ok(())
            }
            _ => Err(ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "unsupported pointer action type",
            )),
        }
    }

    fn pointer_move_coordinates(
        &self,
        action: &serde_json::Map<String, Value>,
        element_origins: &ClassicElementOriginViewportPoints,
    ) -> Result<ClassicViewportPoint, ClassicError> {
        let x = required_finite_number(action, "x")?;
        let y = required_finite_number(action, "y")?;
        match action.get("origin") {
            None => ClassicViewportPoint::new(x, y),
            Some(Value::String(origin)) if origin == "viewport" => ClassicViewportPoint::new(x, y),
            Some(Value::String(origin)) if origin == "pointer" => {
                ClassicViewportPoint::new(self.x + x, self.y + y)
            }
            Some(Value::Object(origin)) => {
                element_origin_viewport_point(origin, element_origins)?.offset(x, y)
            }
            _ => Err(ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "origin must be viewport, pointer, or an element reference",
            )),
        }
    }

    fn next_click_count(&self, button: i32) -> i32 {
        if button == 0
            && self.last_click
                == Some(ClassicPointerClick {
                    x: self.x,
                    y: self.y,
                    button,
                })
        {
            self.click_count.saturating_add(1).max(1)
        } else {
            1
        }
    }

    fn record_click_release(&mut self, button: i32) -> i32 {
        let click_count = self.next_click_count(button);
        if button == 0 {
            self.click_count = click_count;
            self.last_click = Some(ClassicPointerClick {
                x: self.x,
                y: self.y,
                button,
            });
        } else {
            self.click_count = 0;
            self.last_click = None;
        }
        click_count
    }
}

#[derive(Debug, Clone)]
struct ClassicKeyActionSource {
    id: String,
    actions: Vec<Value>,
    pressed: BTreeMap<String, ClassicWebDriverKey>,
    modifiers: u8,
}

impl ClassicKeyActionSource {
    fn parse(
        object: &serde_json::Map<String, Value>,
        action_state: &ClassicActionState,
    ) -> Result<Self, ClassicError> {
        let id = required_action_source_id(object)?;
        if action_state.pointer_sources.contains_key(id)
            || action_state.wheel_sources.contains(id)
            || action_state.none_sources.contains(id)
        {
            return Err(ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "action source id is already used by a different source type",
            ));
        }
        let input_state = action_state
            .key_sources
            .get(id)
            .cloned()
            .unwrap_or_default();
        Ok(Self {
            id: id.to_owned(),
            actions: action_source_actions(object)?,
            pressed: input_state.pressed,
            modifiers: input_state.modifiers,
        })
    }

    fn append_action_command(
        &mut self,
        context: &ClassicDevToolsCommandContext,
        action: &Value,
        commands: &mut Vec<DevToolsCommand>,
        cancel_list: &mut Vec<ClassicActionCancel>,
    ) -> Result<(), ClassicError> {
        let object = action.as_object().ok_or_else(|| {
            ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "key action must be an object",
            )
        })?;
        match required_object_string(object, "type")? {
            "pause" => Ok(()),
            "keyDown" => {
                let key = webdriver_key_for_action(object)?;
                let auto_repeat = self.pressed.contains_key(&key.key);
                if !auto_repeat {
                    cancel_list.push(ClassicActionCancel::Key {
                        source_id: self.id.clone(),
                        key: key.clone(),
                    });
                }
                if let Some(mask) = key.modifier_mask {
                    self.modifiers |= mask;
                }
                self.pressed.insert(key.key.clone(), key.clone());
                let event_key = key.event_key(self.modifiers);
                let input_text = key.input_text(self.modifiers);
                commands.push(dispatch_key_event_command_with_modifiers(
                    context,
                    DevToolsKeyEventType::KeyDown,
                    event_key.as_ref(),
                    &key.code,
                    input_text.as_ref(),
                    self.modifiers,
                    auto_repeat,
                    key.should_insert_text(self.modifiers),
                ));
                Ok(())
            }
            "keyUp" => {
                let key = webdriver_key_for_action(object)?;
                self.pressed.remove(&key.key);
                if let Some(mask) = key.modifier_mask {
                    self.modifiers &= !mask;
                }
                let event_key = key.event_key(self.modifiers);
                commands.push(dispatch_key_event_command_with_modifiers(
                    context,
                    DevToolsKeyEventType::KeyUp,
                    event_key.as_ref(),
                    &key.code,
                    "",
                    self.modifiers,
                    false,
                    false,
                ));
                Ok(())
            }
            _ => Err(ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "unsupported key action type",
            )),
        }
    }
}

#[derive(Debug, Clone)]
struct ClassicWheelActionSource {
    id: String,
    actions: Vec<Value>,
}

impl ClassicWheelActionSource {
    fn parse(
        object: &serde_json::Map<String, Value>,
        action_state: &ClassicActionState,
    ) -> Result<Self, ClassicError> {
        let id = required_action_source_id(object)?;
        if action_state.pointer_sources.contains_key(id)
            || action_state.key_sources.contains_key(id)
            || action_state.none_sources.contains(id)
        {
            return Err(ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "action source id is already used by a different source type",
            ));
        }
        Ok(Self {
            id: id.to_owned(),
            actions: action_source_actions(object)?,
        })
    }

    fn append_action_command(
        &mut self,
        context: &ClassicDevToolsCommandContext,
        action: &Value,
        commands: &mut Vec<DevToolsCommand>,
        element_origins: &ClassicElementOriginViewportPoints,
        viewport_bounds: Option<ClassicViewportBounds>,
    ) -> Result<(), ClassicError> {
        let object = action.as_object().ok_or_else(|| {
            ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "wheel action must be an object",
            )
        })?;
        match required_object_string(object, "type")? {
            "pause" => Ok(()),
            "scroll" => {
                let x_offset = required_i32_number(object, "x")?;
                let y_offset = required_i32_number(object, "y")?;
                let delta_x = required_i32_number(object, "deltaX")?;
                let delta_y = required_i32_number(object, "deltaY")?;
                let point = match object.get("origin") {
                    None => ClassicViewportPoint::new(x_offset, y_offset)?,
                    Some(Value::String(origin)) if origin == "viewport" => {
                        ClassicViewportPoint::new(x_offset, y_offset)?
                    }
                    Some(Value::Object(origin)) => {
                        element_origin_viewport_point(origin, element_origins)?
                            .offset(x_offset, y_offset)?
                    }
                    _ => {
                        return Err(ClassicError::new(
                            ClassicErrorCode::InvalidArgument,
                            "origin must be viewport or an element reference",
                        ));
                    }
                };
                validate_action_target_in_viewport(point, viewport_bounds)?;
                commands.push(dispatch_mouse_event_command_with_delta(
                    context,
                    DevToolsMouseEventType::Wheel,
                    DevToolsPointerType::Mouse,
                    point.x,
                    point.y,
                    0,
                    Some(0),
                    delta_x,
                    delta_y,
                ));
                Ok(())
            }
            _ => Err(ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "unsupported wheel action type",
            )),
        }
    }
}

fn pointer_type_for_mouse_source(pointer_type: &str) -> DevToolsPointerType {
    if pointer_type == "pen" {
        DevToolsPointerType::Pen
    } else {
        DevToolsPointerType::Mouse
    }
}

fn dispatch_mouse_event_command(
    context: &ClassicDevToolsCommandContext,
    event_type: DevToolsMouseEventType,
    pointer_type: DevToolsPointerType,
    x: f64,
    y: f64,
    button: i32,
    buttons: Option<i32>,
) -> DevToolsCommand {
    dispatch_mouse_event_command_with_pointer_properties(
        context,
        event_type,
        pointer_type,
        x,
        y,
        button,
        buttons,
        ClassicPointerEventProperties::mouse_default(),
    )
}

fn dispatch_mouse_event_command_with_pointer_properties(
    context: &ClassicDevToolsCommandContext,
    event_type: DevToolsMouseEventType,
    pointer_type: DevToolsPointerType,
    x: f64,
    y: f64,
    button: i32,
    buttons: Option<i32>,
    properties: ClassicPointerEventProperties,
) -> DevToolsCommand {
    dispatch_mouse_event_command_with_pointer_properties_and_click_count(
        context,
        event_type,
        pointer_type,
        x,
        y,
        button,
        buttons,
        properties,
        default_click_count_for_mouse_event(event_type),
    )
}

fn dispatch_mouse_event_command_with_pointer_properties_and_click_count(
    context: &ClassicDevToolsCommandContext,
    event_type: DevToolsMouseEventType,
    pointer_type: DevToolsPointerType,
    x: f64,
    y: f64,
    button: i32,
    buttons: Option<i32>,
    properties: ClassicPointerEventProperties,
    click_count: i32,
) -> DevToolsCommand {
    dispatch_mouse_event_command_with_delta_and_pointer_properties(
        context,
        event_type,
        pointer_type,
        x,
        y,
        button,
        buttons,
        click_count,
        0.0,
        0.0,
        properties,
    )
}

fn dispatch_mouse_event_command_with_delta(
    context: &ClassicDevToolsCommandContext,
    event_type: DevToolsMouseEventType,
    pointer_type: DevToolsPointerType,
    x: f64,
    y: f64,
    button: i32,
    buttons: Option<i32>,
    delta_x: f64,
    delta_y: f64,
) -> DevToolsCommand {
    dispatch_mouse_event_command_with_delta_and_pointer_properties(
        context,
        event_type,
        pointer_type,
        x,
        y,
        button,
        buttons,
        default_click_count_for_mouse_event(event_type),
        delta_x,
        delta_y,
        ClassicPointerEventProperties::mouse_default(),
    )
}

fn dispatch_mouse_event_command_with_delta_and_pointer_properties(
    context: &ClassicDevToolsCommandContext,
    event_type: DevToolsMouseEventType,
    pointer_type: DevToolsPointerType,
    x: f64,
    y: f64,
    button: i32,
    buttons: Option<i32>,
    click_count: i32,
    delta_x: f64,
    delta_y: f64,
    properties: ClassicPointerEventProperties,
) -> DevToolsCommand {
    DevToolsCommand::DispatchMouseEvent(DevToolsDispatchMouseEventCommand {
        context: context.command_context(),
        event_type,
        pointer_type,
        x,
        y,
        button,
        buttons,
        click_count,
        delta_x,
        delta_y,
        force: properties.pressure,
        tangential_pressure: properties.tangential_pressure,
        tilt_x: properties.tilt_x,
        tilt_y: properties.tilt_y,
        twist: properties.twist,
        modifiers: 0,
    })
}

fn default_click_count_for_mouse_event(event_type: DevToolsMouseEventType) -> i32 {
    match event_type {
        DevToolsMouseEventType::Pressed | DevToolsMouseEventType::Released => 1,
        DevToolsMouseEventType::Moved | DevToolsMouseEventType::Wheel => 0,
    }
}

fn dispatch_touch_event_command(
    context: &ClassicDevToolsCommandContext,
    event_type: DevToolsTouchEventType,
    id: i32,
    x: f64,
    y: f64,
) -> DevToolsCommand {
    DevToolsCommand::DispatchTouchEvent(DevToolsDispatchTouchEventCommand {
        context: context.command_context(),
        event_type,
        touch_points: vec![DevToolsTouchPoint { id, x, y }],
    })
}

fn dispatch_key_event_command_with_modifiers(
    context: &ClassicDevToolsCommandContext,
    event_type: DevToolsKeyEventType,
    key: &str,
    code: &str,
    text: &str,
    modifiers: u8,
    auto_repeat: bool,
    should_insert_text: bool,
) -> DevToolsCommand {
    DevToolsCommand::DispatchKeyEvent(DevToolsDispatchKeyEventCommand {
        context: context.command_context(),
        event_type,
        key: key.to_owned(),
        code: code.to_owned(),
        text: text.to_owned(),
        modifiers,
        auto_repeat,
        should_insert_text,
    })
}

const CLASSIC_MODIFIER_ALT: u8 = 1;
const CLASSIC_MODIFIER_CONTROL: u8 = 2;
const CLASSIC_MODIFIER_META: u8 = 4;
const CLASSIC_MODIFIER_SHIFT: u8 = 8;

#[derive(Debug, Clone, PartialEq)]
struct ClassicWebDriverKey {
    key: String,
    code: String,
    text: String,
    modifier_mask: Option<u8>,
}

impl ClassicWebDriverKey {
    fn text(key: String, code: impl Into<String>, text: String) -> Self {
        Self {
            key,
            code: code.into(),
            text,
            modifier_mask: None,
        }
    }

    fn named(key: impl Into<String>, code: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            code: code.into(),
            text: String::new(),
            modifier_mask: None,
        }
    }

    fn modifier(key: impl Into<String>, code: impl Into<String>, modifier_mask: u8) -> Self {
        Self {
            key: key.into(),
            code: code.into(),
            text: String::new(),
            modifier_mask: Some(modifier_mask),
        }
    }

    fn event_key(&self, modifiers: u8) -> Cow<'_, str> {
        if self.modifier_mask.is_none()
            && modifiers & CLASSIC_MODIFIER_SHIFT != 0
            && let Some(shifted) = shifted_webdriver_action_text(&self.key)
        {
            return Cow::Owned(shifted);
        }
        Cow::Borrowed(&self.key)
    }

    fn input_text(&self, modifiers: u8) -> Cow<'_, str> {
        if !self.should_insert_text(modifiers) {
            return Cow::Borrowed("");
        }
        if modifiers & CLASSIC_MODIFIER_SHIFT != 0
            && let Some(shifted) = shifted_webdriver_action_text(&self.text)
        {
            return Cow::Owned(shifted);
        }
        Cow::Borrowed(&self.text)
    }

    fn should_insert_text(&self, modifiers: u8) -> bool {
        !self.text.is_empty()
            && self.modifier_mask.is_none()
            && modifiers & (CLASSIC_MODIFIER_ALT | CLASSIC_MODIFIER_CONTROL | CLASSIC_MODIFIER_META)
                == 0
    }
}

fn shifted_webdriver_action_text(text: &str) -> Option<String> {
    let mut chars = text.chars();
    let character = chars.next()?;
    if chars.next().is_some() {
        return None;
    }
    let shifted = match character {
        'a'..='z' => character.to_ascii_uppercase(),
        'A'..='Z' => character,
        '0' => ')',
        '1' => '!',
        '2' => '@',
        '3' => '#',
        '4' => '$',
        '5' => '%',
        '6' => '^',
        '7' => '&',
        '8' => '*',
        '9' => '(',
        '`' => '~',
        '-' => '_',
        '=' => '+',
        '[' => '{',
        ']' => '}',
        '\\' => '|',
        ';' => ':',
        '\'' => '"',
        ',' => '<',
        '.' => '>',
        '/' => '?',
        _ => return None,
    };
    Some(shifted.to_string())
}

fn webdriver_key_for_action(
    action: &serde_json::Map<String, Value>,
) -> Result<ClassicWebDriverKey, ClassicError> {
    let value = required_object_string(action, "value")?;
    let mut chars = value.chars();
    let Some(character) = chars.next() else {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "value must be a single character",
        ));
    };
    if chars.next().is_some() {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "value must be a single character",
        ));
    }
    Ok(webdriver_key_for_action_character(character))
}

fn webdriver_key_for_action_character(character: char) -> ClassicWebDriverKey {
    match character {
        '\u{E003}' => ClassicWebDriverKey::named("Backspace", "Backspace"),
        '\u{E004}' => ClassicWebDriverKey::named("Tab", "Tab"),
        '\u{E006}' | '\u{E007}' => ClassicWebDriverKey::named("Enter", "Enter"),
        '\u{E008}' => ClassicWebDriverKey::modifier("Shift", "ShiftLeft", CLASSIC_MODIFIER_SHIFT),
        '\u{E009}' => {
            ClassicWebDriverKey::modifier("Control", "ControlLeft", CLASSIC_MODIFIER_CONTROL)
        }
        '\u{E00A}' => ClassicWebDriverKey::modifier("Alt", "AltLeft", CLASSIC_MODIFIER_ALT),
        '\u{E00C}' => ClassicWebDriverKey::named("Escape", "Escape"),
        '\u{E00D}' => ClassicWebDriverKey::text(" ".to_owned(), "Space", " ".to_owned()),
        '\u{E00E}' => ClassicWebDriverKey::named("PageUp", "PageUp"),
        '\u{E00F}' => ClassicWebDriverKey::named("PageDown", "PageDown"),
        '\u{E010}' => ClassicWebDriverKey::named("End", "End"),
        '\u{E011}' => ClassicWebDriverKey::named("Home", "Home"),
        '\u{E012}' => ClassicWebDriverKey::named("ArrowLeft", "ArrowLeft"),
        '\u{E013}' => ClassicWebDriverKey::named("ArrowUp", "ArrowUp"),
        '\u{E014}' => ClassicWebDriverKey::named("ArrowRight", "ArrowRight"),
        '\u{E015}' => ClassicWebDriverKey::named("ArrowDown", "ArrowDown"),
        '\u{E016}' => ClassicWebDriverKey::named("Insert", "Insert"),
        '\u{E017}' => ClassicWebDriverKey::named("Delete", "Delete"),
        '\u{E018}' => ClassicWebDriverKey::text(";".to_owned(), "Semicolon", ";".to_owned()),
        '\u{E019}' => ClassicWebDriverKey::text("=".to_owned(), "Equal", "=".to_owned()),
        '\u{E01A}' => ClassicWebDriverKey::text("0".to_owned(), "Numpad0", "0".to_owned()),
        '\u{E01B}' => ClassicWebDriverKey::text("1".to_owned(), "Numpad1", "1".to_owned()),
        '\u{E01C}' => ClassicWebDriverKey::text("2".to_owned(), "Numpad2", "2".to_owned()),
        '\u{E01D}' => ClassicWebDriverKey::text("3".to_owned(), "Numpad3", "3".to_owned()),
        '\u{E01E}' => ClassicWebDriverKey::text("4".to_owned(), "Numpad4", "4".to_owned()),
        '\u{E01F}' => ClassicWebDriverKey::text("5".to_owned(), "Numpad5", "5".to_owned()),
        '\u{E020}' => ClassicWebDriverKey::text("6".to_owned(), "Numpad6", "6".to_owned()),
        '\u{E021}' => ClassicWebDriverKey::text("7".to_owned(), "Numpad7", "7".to_owned()),
        '\u{E022}' => ClassicWebDriverKey::text("8".to_owned(), "Numpad8", "8".to_owned()),
        '\u{E023}' => ClassicWebDriverKey::text("9".to_owned(), "Numpad9", "9".to_owned()),
        '\u{E024}' => ClassicWebDriverKey::text("*".to_owned(), "NumpadMultiply", "*".to_owned()),
        '\u{E025}' => ClassicWebDriverKey::text("+".to_owned(), "NumpadAdd", "+".to_owned()),
        '\u{E026}' => ClassicWebDriverKey::text(",".to_owned(), "NumpadComma", ",".to_owned()),
        '\u{E027}' => ClassicWebDriverKey::text("-".to_owned(), "NumpadSubtract", "-".to_owned()),
        '\u{E028}' => ClassicWebDriverKey::text(".".to_owned(), "NumpadDecimal", ".".to_owned()),
        '\u{E029}' => ClassicWebDriverKey::text("/".to_owned(), "NumpadDivide", "/".to_owned()),
        '\u{E031}' => ClassicWebDriverKey::named("F1", "F1"),
        '\u{E032}' => ClassicWebDriverKey::named("F2", "F2"),
        '\u{E033}' => ClassicWebDriverKey::named("F3", "F3"),
        '\u{E034}' => ClassicWebDriverKey::named("F4", "F4"),
        '\u{E035}' => ClassicWebDriverKey::named("F5", "F5"),
        '\u{E036}' => ClassicWebDriverKey::named("F6", "F6"),
        '\u{E037}' => ClassicWebDriverKey::named("F7", "F7"),
        '\u{E038}' => ClassicWebDriverKey::named("F8", "F8"),
        '\u{E039}' => ClassicWebDriverKey::named("F9", "F9"),
        '\u{E03A}' => ClassicWebDriverKey::named("F10", "F10"),
        '\u{E03B}' => ClassicWebDriverKey::named("F11", "F11"),
        '\u{E03C}' => ClassicWebDriverKey::named("F12", "F12"),
        '\u{E03D}' => ClassicWebDriverKey::modifier("Meta", "MetaLeft", CLASSIC_MODIFIER_META),
        _ => webdriver_key_for_character(character),
    }
}

fn webdriver_key_for_character(character: char) -> ClassicWebDriverKey {
    match character {
        '\n' | '\r' => ClassicWebDriverKey::text("Enter".to_owned(), "Enter", "\n".to_owned()),
        '\t' => ClassicWebDriverKey::text("Tab".to_owned(), "Tab", "\t".to_owned()),
        _ => {
            let text = character.to_string();
            ClassicWebDriverKey::text(text.clone(), "", text)
        }
    }
}

pub fn element_center_from_geometry(
    geometry: &DevToolsDomGeometryResult,
) -> Result<ClassicViewportPoint, ClassicError> {
    let Some(quad) = geometry_border_quad(geometry) else {
        return Err(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "element has no clickable geometry",
        ));
    };
    let points = &quad.points;
    if points.len() < 8 {
        return Err(ClassicError::new(
            ClassicErrorCode::UnknownError,
            "element has invalid clickable geometry",
        ));
    }
    let x = (points[0] + points[2] + points[4] + points[6]) / 4.0;
    let y = (points[1] + points[3] + points[5] + points[7]) / 4.0;
    ClassicViewportPoint::new(x, y).map_err(|_| {
        ClassicError::new(
            ClassicErrorCode::UnknownError,
            "element has invalid clickable geometry",
        )
    })
}

fn element_origin_reference(
    origin: &serde_json::Map<String, Value>,
) -> Result<String, ClassicError> {
    let Some(element_id) = origin.get(CLASSIC_ELEMENT_REFERENCE_KEY) else {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "element origin must contain an element reference",
        ));
    };
    let Some(element_id) = element_id.as_str() else {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "element origin reference must be a string",
        ));
    };
    cdp_node_id_from_classic_element_id(element_id)?;
    Ok(element_id.to_owned())
}

fn element_origin_viewport_point(
    origin: &serde_json::Map<String, Value>,
    element_origins: &ClassicElementOriginViewportPoints,
) -> Result<ClassicViewportPoint, ClassicError> {
    let element_id = element_origin_reference(origin)?;
    element_origins.get(&element_id).copied().ok_or_else(|| {
        ClassicError::new(
            ClassicErrorCode::UnknownError,
            "element origin geometry was not resolved",
        )
    })
}

fn validate_action_target_in_viewport(
    point: ClassicViewportPoint,
    viewport_bounds: Option<ClassicViewportBounds>,
) -> Result<(), ClassicError> {
    let Some(viewport_bounds) = viewport_bounds else {
        return Ok(());
    };
    if viewport_bounds.contains(point) {
        Ok(())
    } else {
        Err(ClassicError::new(
            ClassicErrorCode::MoveTargetOutOfBounds,
            "move target is outside the viewport",
        ))
    }
}

fn pointer_button_code(action: &serde_json::Map<String, Value>) -> Result<i32, ClassicError> {
    let Some(button) = action.get("button").and_then(Value::as_i64) else {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "button must be an integer",
        ));
    };
    if !(0..=4).contains(&button) {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "button must be between 0 and 4",
        ));
    }
    Ok(button as i32)
}

fn pointer_button_mask(button: i32) -> i32 {
    match button {
        0 => 1,
        1 => 4,
        2 => 2,
        3 => 8,
        4 => 16,
        _ => 0,
    }
}

fn pointer_event_properties(
    action: &serde_json::Map<String, Value>,
) -> Result<ClassicPointerEventProperties, ClassicError> {
    let _width = optional_finite_number_in_range(action, "width", 1.0, 0.0, f64::INFINITY)?;
    let _height = optional_finite_number_in_range(action, "height", 1.0, 0.0, f64::INFINITY)?;
    let defaults = ClassicPointerEventProperties::pointer_action_default();
    Ok(ClassicPointerEventProperties {
        pressure: optional_finite_number_in_range(action, "pressure", defaults.pressure, 0.0, 1.0)?,
        tangential_pressure: optional_finite_number_in_range(
            action,
            "tangentialPressure",
            defaults.tangential_pressure,
            -1.0,
            1.0,
        )?,
        tilt_x: f64::from(optional_i32_in_range(
            action,
            "tiltX",
            defaults.tilt_x as i32,
            -90,
            90,
        )?),
        tilt_y: f64::from(optional_i32_in_range(
            action,
            "tiltY",
            defaults.tilt_y as i32,
            -90,
            90,
        )?),
        twist: f64::from(optional_i32_in_range(
            action,
            "twist",
            defaults.twist as i32,
            0,
            359,
        )?),
    })
}

fn optional_finite_number_in_range(
    params: &serde_json::Map<String, Value>,
    field: &str,
    default: f64,
    min: f64,
    max: f64,
) -> Result<f64, ClassicError> {
    let Some(value) = params.get(field) else {
        return Ok(default);
    };
    let Some(value) = value.as_f64() else {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            format!("{field} must be a number"),
        ));
    };
    if !value.is_finite() {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            format!("{field} must be finite"),
        ));
    }
    if value < min || value > max {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            format!("{field} is outside the supported range"),
        ));
    }
    Ok(value)
}

fn optional_i32_in_range(
    params: &serde_json::Map<String, Value>,
    field: &str,
    default: i32,
    min: i32,
    max: i32,
) -> Result<i32, ClassicError> {
    let Some(value) = params.get(field) else {
        return Ok(default);
    };
    let Some(value) = value.as_i64() else {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            format!("{field} must be an integer"),
        ));
    };
    let value: i32 = value.try_into().map_err(|_| {
        ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            format!("{field} must be a 32-bit integer"),
        )
    })?;
    if value < min || value > max {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            format!("{field} is outside the supported range"),
        ));
    }
    Ok(value)
}

fn required_action_source_id(
    params: &serde_json::Map<String, Value>,
) -> Result<&str, ClassicError> {
    let id = required_object_string(params, "id")?;
    if id.is_empty() {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "action source id must be non-empty",
        ));
    }
    Ok(id)
}

fn action_source_actions(
    params: &serde_json::Map<String, Value>,
) -> Result<Vec<Value>, ClassicError> {
    params
        .get("actions")
        .and_then(Value::as_array)
        .cloned()
        .ok_or_else(|| {
            ClassicError::new(
                ClassicErrorCode::InvalidArgument,
                "source actions must be an array",
            )
        })
}

fn optional_duration_ms(params: &serde_json::Map<String, Value>) -> Result<u64, ClassicError> {
    let Some(value) = params.get("duration") else {
        return Ok(0);
    };
    value.as_u64().ok_or_else(|| {
        ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            "duration must be a non-negative integer",
        )
    })
}

fn required_finite_number(
    params: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<f64, ClassicError> {
    let Some(value) = params.get(field).and_then(Value::as_f64) else {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            format!("{field} must be a number"),
        ));
    };
    if !value.is_finite() {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            format!("{field} must be finite"),
        ));
    }
    Ok(value)
}

fn required_i32_number(
    params: &serde_json::Map<String, Value>,
    field: &str,
) -> Result<f64, ClassicError> {
    let Some(value) = params.get(field).and_then(Value::as_i64) else {
        return Err(ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            format!("{field} must be an integer"),
        ));
    };
    let value: i32 = value.try_into().map_err(|_| {
        ClassicError::new(
            ClassicErrorCode::InvalidArgument,
            format!("{field} must be a 32-bit integer"),
        )
    })?;
    Ok(f64::from(value))
}
