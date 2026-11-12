use super::*;
use moli_protocol::automation::{
    DevToolsDomGeometryCommand, DevToolsDomGeometryOperation, DevToolsDomNodeReference,
    DevToolsFrameId, DevToolsGetFrameOwnerCommand,
};
use moli_protocol::conn::ProtocolPageHostScope;
use moli_protocol_webdriver_classic::ClassicViewportPoint;
use std::{cell::RefCell, rc::Rc};

/// Device state and the actions queue belong to the top-level traversable,
/// rather than to the frame used by an individual perform/release command.
pub(super) struct BidiInputState {
    host: ProtocolPageHostScope,
    actions: ClassicActionState,
    commands: VecDeque<u64>,
}

pub(super) struct BidiPendingInput {
    host: ProtocolPageHostScope,
    top_level: String,
    context: String,
    device: Rc<RefCell<BidiInputState>>,
    queued: bool,
    phase: InputPhase,
}

enum InputPhase {
    Begin(BidiInputCommand),
    Viewport(BidiInputCommand),
    FrameOwner(InputSetup),
    FrameGeometry(InputSetup),
    Origin {
        setup: InputSetup,
        classic_id: String,
    },
    Actions {
        sequence: Option<InputSequence>,
        commands: VecDeque<AutomationCommand>,
        duration_ms: u64,
    },
}

struct InputSequence {
    ticks: VecDeque<Value>,
    viewport: ClassicViewportBounds,
    points: ClassicElementOriginViewportPoints,
}

struct InputSetup {
    params: Value,
    viewport: ClassicViewportBounds,
    origins: VecDeque<(String, String)>,
    points: ClassicElementOriginViewportPoints,
}

fn context(session_id: &str, context: &str) -> AutomationContext {
    AutomationContext {
        protocol: FrontendProtocol::WebDriverBidi,
        session_id: Some(DevToolsSessionId::from(session_id)),
        target_id: Some(DevToolsTargetId::from(context)),
        browser_context_id: None,
    }
}

pub(super) async fn start_bidi_input_command(
    scheduler: &mut CdpScheduler,
    receivers: &mut CdpSchedulerEventReceivers,
    sender: &mpsc::UnboundedSender<AutomationWorkflowReady>,
    bidi: &BidiConnectionState,
    states: &mut BTreeMap<String, Rc<RefCell<BidiInputState>>>,
    dispatch: BidiInputCommandDispatch,
) -> BidiDevToolsCommandStart {
    let event_context = Some(dispatch.context.clone());
    let failed = |response| {
        BidiDevToolsCommandStart::Complete(BidiDevToolsCommandOutput {
            response,
            event_context: event_context.clone(),
            event_sources: Vec::new(),
            post_response_event_sources: Vec::new(),
        })
    };
    // setFiles has no actions-queue dependency. It uses the same owned Native
    // command machinery as script and navigation, including Document admission.
    if let BidiInputCommand::SetFiles { params } = &dispatch.command {
        let (shared_id, paths) = match parse_bidi_set_files_params(params) {
            Ok(parsed) => parsed,
            Err(message) => {
                return failed(error_response(
                    Some(dispatch.id),
                    BidiErrorCode::InvalidArgument,
                    &message,
                ));
            }
        };
        let files = match selected_files_from_paths(&paths, "input.setFiles") {
            Ok(files) => files,
            Err(error) => return failed(bidi_response_from_devtools_error(dispatch.id, error)),
        };
        let command = AutomationCommand::SetFileInputFiles(DevToolsSetFileInputFilesCommand {
            context: context(&dispatch.session_id, &dispatch.context),
            object_id: DevToolsRemoteHandleId::from(shared_id.as_str()),
            files,
            append: false,
        });
        return start_bidi_automation_command(
            scheduler,
            receivers,
            sender,
            bidi,
            BidiDevToolsCommandDispatch {
                id: dispatch.id,
                session_id: dispatch.session_id,
                command,
            },
            None,
        )
        .await;
    }
    let Some(owner) = scheduler.browsing_context_owner(&dispatch.context) else {
        return failed(bidi_response_from_devtools_error(dispatch.id, closed()));
    };
    let command_context = context(&dispatch.session_id, &owner.page);
    let Some(host) = scheduler.automation_page_host_scope(&command_context) else {
        return failed(bidi_response_from_devtools_error(dispatch.id, closed()));
    };
    let state = states.entry(owner.page.clone()).or_insert_with(|| {
        Rc::new(RefCell::new(BidiInputState {
            host: host.clone(),
            actions: ClassicActionState::default(),
            commands: VecDeque::new(),
        }))
    });
    if !state.borrow().host.same_host(&host) {
        *state = Rc::new(RefCell::new(BidiInputState {
            host: host.clone(),
            actions: ClassicActionState::default(),
            commands: VecDeque::new(),
        }));
    }
    let queued = matches!(dispatch.command, BidiInputCommand::PerformActions { .. });
    if queued {
        state.borrow_mut().commands.push_back(dispatch.id);
    }
    let device = Rc::clone(state);
    BidiDevToolsCommandStart::Pending(Box::new(BidiPendingCommand {
        command_method: None,
        command_params: None,
        command_channel: None,
        pending_navigation_candidate: None,
        completion: BidiDevToolsCommandCompletion {
            id: dispatch.id,
            command_context,
            navigation_wait: None,
            event_sources: BidiDevToolsEventSources::default(),
            event_context,
        },
        input: Some(BidiPendingInput {
            host,
            device,
            queued,
            top_level: owner.page,
            context: dispatch.context,
            phase: InputPhase::Begin(dispatch.command),
        }),
        stage: Some(BidiPendingCommandStage::WaitingForInputTurn),
    }))
}

fn closed() -> DevToolsError {
    DevToolsError::new(
        DevToolsErrorKind::NoSuchTarget,
        "Input browsing context is no longer available",
    )
}
fn unexpected() -> DevToolsError {
    DevToolsError::new(
        DevToolsErrorKind::Internal,
        "input preparation returned an unexpected result",
    )
}

impl BidiSocketActor {
    pub(super) fn next_input_deadline(&self) -> Option<Instant> {
        self.pending_commands
            .iter()
            .filter_map(|command| match command.stage {
                Some(BidiPendingCommandStage::InputPause(deadline)) => Some(deadline),
                _ => None,
            })
            .min()
    }

    pub(super) fn input_owner_is_current(
        &self,
        scheduler: &mut CdpScheduler,
        command: &BidiPendingCommand,
    ) -> bool {
        let input = command.input.as_ref().expect("an input continuation");
        scheduler.automation_page_host_can_start_operation(&input.host)
            && scheduler
                .browsing_context_owner(&input.context)
                .is_some_and(|owner| owner.page == input.top_level)
    }

    pub(super) fn input_turn_is_available(&self, command: &BidiPendingCommand) -> bool {
        let input = command.input.as_ref().expect("an input continuation");
        !input.queued || input.device.borrow().commands.front() == Some(&command.completion.id)
    }

    pub(super) async fn begin_input_command(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        mut command: BidiPendingCommand,
    ) -> bool {
        if !self.input_owner_is_current(scheduler, &command) {
            return self
                .fail_input_command(
                    scheduler,
                    receivers,
                    command,
                    BidiInputPreparationError::DevTools(closed()),
                )
                .await;
        }
        let input = command.input.as_mut().expect("input");
        let InputPhase::Begin(action) = std::mem::replace(
            &mut input.phase,
            InputPhase::Begin(BidiInputCommand::ReleaseActions),
        ) else {
            unreachable!("input starts once");
        };
        input.phase = InputPhase::Viewport(action);
        let native = AutomationCommand::GetLayoutMetrics(DevToolsGetLayoutMetricsCommand {
            context: command.completion.command_context.clone(),
        });
        self.start_input_native(scheduler, receivers, command, native)
            .await
    }

    async fn start_input_native(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        mut command: BidiPendingCommand,
        native: AutomationCommand,
    ) -> bool {
        let (stage, output) = match scheduler
            .start_owned_automation_command_progress(
                receivers,
                &self.automation_workflow_ready_tx,
                native,
                None,
            )
            .await
        {
            Ok(AutomationCommandProgress::Complete(execution)) => (
                BidiPendingCommandStage::NativeComplete(execution),
                ProtocolOutputSequence::empty(),
            ),
            Ok(AutomationCommandProgress::Pending {
                pending,
                protocol_output,
            }) => (BidiPendingCommandStage::Executing(pending), protocol_output),
            Err(admission) => (
                BidiPendingCommandStage::WaitingForDocument(admission),
                ProtocolOutputSequence::empty(),
            ),
        };
        command.stage = Some(stage);
        self.pending_commands.push_back(command);
        self.publish_pending_command_output(scheduler, receivers, output, None)
            .await
    }

    pub(super) async fn advance_input_command(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        mut command: BidiPendingCommand,
        execution: crate::cdp_scheduler::AutomationExecution,
    ) -> bool {
        if !self
            .publish_pending_command_output(scheduler, receivers, execution.protocol_output, None)
            .await
        {
            return false;
        }
        if !self.input_owner_is_current(scheduler, &command) {
            return self
                .fail_input_command(
                    scheduler,
                    receivers,
                    command,
                    BidiInputPreparationError::DevTools(closed()),
                )
                .await;
        }
        let result = match execution.result {
            Ok(result) => result,
            Err(error) => {
                return self
                    .fail_input_command(
                        scheduler,
                        receivers,
                        command,
                        BidiInputPreparationError::DevTools(error),
                    )
                    .await;
            }
        };
        let input = command.input.as_mut().expect("input");
        match std::mem::replace(
            &mut input.phase,
            InputPhase::Begin(BidiInputCommand::ReleaseActions),
        ) {
            InputPhase::Viewport(action) => {
                let AutomationResult::LayoutMetrics(metrics) = result else {
                    return self
                        .fail_input_command(
                            scheduler,
                            receivers,
                            command,
                            BidiInputPreparationError::DevTools(unexpected()),
                        )
                        .await;
                };
                let viewport = ClassicViewportBounds::new(
                    metrics.layout_viewport_width,
                    metrics.layout_viewport_height,
                );
                match action {
                    BidiInputCommand::PerformActions { mut params } => {
                        let origins =
                            match rewrite_bidi_element_origins_for_classic_actions(&mut params) {
                                Ok(origins) => origins.into_iter().collect(),
                                Err(error) => {
                                    return self
                                        .fail_input_command(scheduler, receivers, command, error)
                                        .await;
                                }
                            };
                        let setup = InputSetup {
                            params,
                            viewport,
                            origins,
                            points: BTreeMap::new(),
                        };
                        if input.context != input.top_level
                            && has_viewport_coordinates(&setup.params)
                        {
                            let native =
                                AutomationCommand::GetFrameOwner(DevToolsGetFrameOwnerCommand {
                                    context: command.completion.command_context.clone(),
                                    frame_id: DevToolsFrameId::from(input.context.as_str()),
                                });
                            input.phase = InputPhase::FrameOwner(setup);
                            self.start_input_native(scheduler, receivers, command, native)
                                .await
                        } else {
                            self.prepare_input_origins(scheduler, receivers, command, setup)
                                .await
                        }
                    }
                    BidiInputCommand::ReleaseActions => {
                        let context = ClassicDevToolsCommandContext::with_protocol_and_target_id(
                            FrontendProtocol::WebDriverBidi,
                            command
                                .completion
                                .command_context
                                .session_id
                                .as_ref()
                                .unwrap()
                                .as_str(),
                            &input.top_level,
                        );
                        let commands = {
                            let mut state = input.device.borrow_mut();
                            release_actions_commands(&context, &mut state.actions)
                        };
                        input.phase = InputPhase::Actions {
                            sequence: None,
                            commands: commands.into(),
                            duration_ms: 0,
                        };
                        self.continue_input_actions(scheduler, receivers, command)
                            .await
                    }
                    BidiInputCommand::SetFiles { .. } => {
                        unreachable!("setFiles uses an independent Native command")
                    }
                }
            }
            InputPhase::FrameOwner(setup) => {
                let AutomationResult::GetFrameOwner(owner) = result else {
                    return self
                        .fail_input_command(
                            scheduler,
                            receivers,
                            command,
                            BidiInputPreparationError::DevTools(unexpected()),
                        )
                        .await;
                };
                input.phase = InputPhase::FrameGeometry(setup);
                let native = AutomationCommand::DomGeometry(DevToolsDomGeometryCommand {
                    context: command.completion.command_context.clone(),
                    reference: DevToolsDomNodeReference::BackendNodeId(owner.backend_node_id),
                    operation: DevToolsDomGeometryOperation::GetBoxModel,
                });
                self.start_input_native(scheduler, receivers, command, native)
                    .await
            }
            InputPhase::FrameGeometry(mut setup) => {
                let AutomationResult::DomGeometry(geometry) = result else {
                    return self
                        .fail_input_command(
                            scheduler,
                            receivers,
                            command,
                            BidiInputPreparationError::DevTools(unexpected()),
                        )
                        .await;
                };
                let Some(model) = geometry.box_model else {
                    return self
                        .fail_input_command(
                            scheduler,
                            receivers,
                            command,
                            BidiInputPreparationError::DevTools(unexpected()),
                        )
                        .await;
                };
                let [x, y, ..] = model.content.points.as_slice() else {
                    return self
                        .fail_input_command(
                            scheduler,
                            receivers,
                            command,
                            BidiInputPreparationError::DevTools(unexpected()),
                        )
                        .await;
                };
                offset_viewport_coordinates(&mut setup.params, *x, *y);
                self.prepare_input_origins(scheduler, receivers, command, setup)
                    .await
            }
            InputPhase::Origin {
                mut setup,
                classic_id,
            } => {
                let AutomationResult::DomGeometry(geometry) = result else {
                    return self
                        .fail_input_command(
                            scheduler,
                            receivers,
                            command,
                            BidiInputPreparationError::DevTools(unexpected()),
                        )
                        .await;
                };
                let point: ClassicViewportPoint = match element_center_from_geometry(&geometry) {
                    Ok(point) => point,
                    Err(error) => {
                        return self
                            .fail_input_command(
                                scheduler,
                                receivers,
                                command,
                                BidiInputPreparationError::Classic(error),
                            )
                            .await;
                    }
                };
                setup.points.insert(classic_id, point);
                self.prepare_input_origins(scheduler, receivers, command, setup)
                    .await
            }
            phase @ InputPhase::Actions { .. } => {
                input.phase = phase;
                if !matches!(result, AutomationResult::Empty) {
                    return self
                        .fail_input_command(
                            scheduler,
                            receivers,
                            command,
                            BidiInputPreparationError::DevTools(unexpected()),
                        )
                        .await;
                }
                self.continue_input_actions(scheduler, receivers, command)
                    .await
            }
            InputPhase::Begin(_) => unreachable!("begin has no Native wait"),
        }
    }

    async fn prepare_input_origins(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        mut command: BidiPendingCommand,
        mut setup: InputSetup,
    ) -> bool {
        let input = command.input.as_mut().expect("input");
        if let Some((shared_id, classic_id)) = setup.origins.pop_front() {
            let native = AutomationCommand::DomObjectReference(DevToolsDomObjectReferenceCommand {
                context: context(
                    command
                        .completion
                        .command_context
                        .session_id
                        .as_ref()
                        .unwrap()
                        .as_str(),
                    &input.top_level,
                ),
                object_id: DevToolsRemoteHandleId::from(shared_id.as_str()),
                operation: DevToolsDomObjectReferenceOperation::GetBoxModel,
            });
            input.phase = InputPhase::Origin { setup, classic_id };
            return self
                .start_input_native(scheduler, receivers, command, native)
                .await;
        }
        let context = ClassicDevToolsCommandContext::with_protocol_and_target_id(
            FrontendProtocol::WebDriverBidi,
            command
                .completion
                .command_context
                .session_id
                .as_ref()
                .unwrap()
                .as_str(),
            &input.top_level,
        );
        // Validate the complete request without applying future key/pointer
        // transitions to the live device. Each tick commits only when dispatched.
        let mut validation = input.device.borrow().actions.clone();
        if let Err(error) = perform_actions_ticks_with_state_and_viewport(
            &context,
            &setup.params,
            &setup.points,
            Some(setup.viewport),
            &mut validation,
        ) {
            return self
                .fail_input_command(
                    scheduler,
                    receivers,
                    command,
                    BidiInputPreparationError::Classic(error),
                )
                .await;
        }
        input.phase = InputPhase::Actions {
            sequence: Some(InputSequence {
                ticks: input_ticks(&setup.params),
                viewport: setup.viewport,
                points: setup.points,
            }),
            commands: VecDeque::new(),
            duration_ms: 0,
        };
        self.continue_input_actions(scheduler, receivers, command)
            .await
    }

    pub(super) async fn continue_input_actions(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        mut command: BidiPendingCommand,
    ) -> bool {
        if !self.input_owner_is_current(scheduler, &command) {
            return self
                .fail_input_command(
                    scheduler,
                    receivers,
                    command,
                    BidiInputPreparationError::DevTools(closed()),
                )
                .await;
        }
        let input = command.input.as_mut().expect("input");
        let InputPhase::Actions {
            sequence,
            commands,
            duration_ms,
        } = &mut input.phase
        else {
            unreachable!("only actions have ticks");
        };
        loop {
            if let Some(native) = commands.pop_front() {
                return self
                    .start_input_native(scheduler, receivers, command, native)
                    .await;
            }
            let duration = std::mem::take(duration_ms);
            if duration > 0 {
                command.stage = Some(BidiPendingCommandStage::InputPause(
                    Instant::now() + Duration::from_millis(duration),
                ));
                self.pending_commands.push_back(command);
                return true;
            }
            let Some(params) = sequence
                .as_mut()
                .and_then(|sequence| sequence.ticks.pop_front())
            else {
                self.release_input_turn(&command);
                command.input = None;
                return self
                    .finish_pending_command(
                        scheduler,
                        receivers,
                        command,
                        crate::cdp_scheduler::AutomationExecution {
                            result: Ok(AutomationResult::Empty),
                            protocol_output: ProtocolOutputSequence::empty(),
                        },
                    )
                    .await;
            };
            let sequence = sequence.as_ref().expect("an actions sequence");
            let context = ClassicDevToolsCommandContext::with_protocol_and_target_id(
                FrontendProtocol::WebDriverBidi,
                command
                    .completion
                    .command_context
                    .session_id
                    .as_ref()
                    .unwrap()
                    .as_str(),
                &input.top_level,
            );
            let planned = perform_actions_ticks_with_state_and_viewport(
                &context,
                &params,
                &sequence.points,
                Some(sequence.viewport),
                &mut input.device.borrow_mut().actions,
            );
            let tick = match planned {
                Ok(mut ticks) => ticks.pop().expect("one input tick"),
                Err(error) => {
                    return self
                        .fail_input_command(
                            scheduler,
                            receivers,
                            command,
                            BidiInputPreparationError::Classic(error),
                        )
                        .await;
                }
            };
            *commands = tick.commands.into();
            *duration_ms = tick.duration_ms;
        }
    }

    fn release_input_turn(&mut self, command: &BidiPendingCommand) {
        let input = command.input.as_ref().expect("input");
        if input.queued {
            let mut state = input.device.borrow_mut();
            if let Some(index) = state
                .commands
                .iter()
                .position(|id| *id == command.completion.id)
            {
                state.commands.remove(index);
            }
        } else if self
            .input_action_states
            .get(&input.top_level)
            .is_some_and(|state| Rc::ptr_eq(state, &input.device))
        {
            // Reset this generation only. An already accepted performActions
            // keeps its own state/queue; a later request gets fresh devices.
            self.input_action_states.remove(&input.top_level);
        }
    }

    pub(super) async fn fail_closed_input_command(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        command: BidiPendingCommand,
    ) -> bool {
        self.fail_input_command(
            scheduler,
            receivers,
            command,
            BidiInputPreparationError::DevTools(closed()),
        )
        .await
    }

    async fn fail_input_command(
        &mut self,
        scheduler: &mut CdpScheduler,
        receivers: &mut CdpSchedulerEventReceivers,
        mut command: BidiPendingCommand,
        error: BidiInputPreparationError,
    ) -> bool {
        self.release_input_turn(&command);
        let response = match error {
            BidiInputPreparationError::Classic(error) => {
                bidi_response_from_classic_input_error(command.completion.id, error)
            }
            BidiInputPreparationError::DevTools(error) => {
                bidi_response_from_devtools_error(command.completion.id, error)
            }
        };
        let output = BidiDevToolsCommandOutput {
            response,
            event_sources: std::mem::take(&mut command.completion.event_sources).into_sources(),
            post_response_event_sources: Vec::new(),
            event_context: command.completion.event_context.clone(),
        };
        send_completed_bidi_command(
            &mut self.socket,
            scheduler,
            receivers,
            &mut self.bidi,
            &mut self.pending_navigation_responses,
            command,
            output,
        )
        .await
    }
}

pub(super) async fn wait_for_bidi_input_deadline(deadline: Option<Instant>) {
    match deadline {
        Some(deadline) => sleep_until(deadline).await,
        None => std::future::pending().await,
    }
}

fn input_ticks(params: &Value) -> VecDeque<Value> {
    // Copy source metadata once, rather than cloning its entire action list
    // for every tick. The original request has already been validated.
    let sources = params["actions"]
        .as_array()
        .expect("validated action sources")
        .iter()
        .map(|source| {
            let mut header = source.as_object().unwrap().clone();
            let actions = header
                .remove("actions")
                .unwrap()
                .as_array()
                .unwrap()
                .clone();
            (header, actions)
        })
        .collect::<Vec<_>>();
    let count = sources
        .iter()
        .map(|(_, actions)| actions.len())
        .max()
        .unwrap_or(0);
    (0..count)
        .map(|index| {
            let actions = sources
                .iter()
                .map(|(header, actions)| {
                    let mut source = header.clone();
                    source.insert(
                        "actions".into(),
                        Value::Array(actions.get(index).cloned().into_iter().collect()),
                    );
                    Value::Object(source)
                })
                .collect::<Vec<_>>();
            json!({"actions": actions})
        })
        .collect()
}

fn has_viewport_coordinates(params: &Value) -> bool {
    params["actions"].as_array().is_some_and(|sources| {
        sources.iter().any(|source| {
            source["actions"].as_array().is_some_and(|actions| {
                actions.iter().any(|action| {
                    matches!(action["type"].as_str(), Some("pointerMove" | "scroll"))
                        && action
                            .get("origin")
                            .is_none_or(|origin| origin == "viewport")
                })
            })
        })
    })
}

fn offset_viewport_coordinates(params: &mut Value, x: f64, y: f64) {
    if let Some(sources) = params["actions"].as_array_mut() {
        for source in sources {
            if let Some(actions) = source["actions"].as_array_mut() {
                for action in actions {
                    if matches!(action["type"].as_str(), Some("pointerMove" | "scroll"))
                        && action
                            .get("origin")
                            .is_none_or(|origin| origin == "viewport")
                    {
                        for (axis, offset) in [("x", x), ("y", y)] {
                            if let Some(value) = action[axis].as_f64() {
                                action[axis] = json!(value + offset);
                            }
                        }
                    }
                }
            }
        }
    }
}
