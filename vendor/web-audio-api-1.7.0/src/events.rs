#[cfg(feature = "diagnostics")]
use crate::context::AudioContextDiagnostics;
use crate::context::ConcreteBaseAudioContext;
use crate::context::{AudioContextState, AudioNodeId};
use crate::{AudioBuffer, AudioRenderCapacityEvent};

use std::any::Any;
use std::collections::HashMap;
use std::hash::Hash;
use std::ops::ControlFlow;
use std::sync::{Arc, Mutex};

use crossbeam_channel::Receiver;

/// The Event interface
#[derive(Debug, Clone)]
#[non_exhaustive]
pub struct Event {
    pub type_: &'static str,
}

#[derive(Hash, Eq, PartialEq, Debug)]
pub(crate) enum EventType {
    Ended(AudioNodeId),
    SinkChange,
    StateChange,
    RenderCapacity,
    ProcessorError(AudioNodeId),
    #[cfg(feature = "diagnostics")]
    Diagnostics,
    Message(AudioNodeId),
    Complete,
    AudioProcessing(AudioNodeId),
}

/// The Error Event interface
#[non_exhaustive]
#[derive(Debug)]
pub struct ErrorEvent {
    /// The error message
    pub message: String,
    /// The object with which panic was originally invoked.
    pub error: Box<dyn Any + Send>,
    /// Inherits from this base Event
    pub event: Event,
}

/// The AudioProcessingEvent interface
#[non_exhaustive]
#[derive(Debug)]
pub struct AudioProcessingEvent {
    /// The input buffer
    pub input_buffer: AudioBuffer,
    /// The output buffer
    pub output_buffer: AudioBuffer,
    /// The time when the audio will be played in the same time coordinate system as the
    /// AudioContext's currentTime.
    pub playback_time: f64,
    pub(crate) registration: Option<(ConcreteBaseAudioContext, AudioNodeId)>,
}

impl Drop for AudioProcessingEvent {
    fn drop(&mut self) {
        if let Some((context, id)) = self.registration.take() {
            let wrapped = crate::message::ControlMessage::NodeMessage {
                id,
                msg: llq::Node::new(Box::new(self.output_buffer.clone())),
            };
            context.send_control_msg(wrapped);
        }
    }
}

/// The OfflineAudioCompletionEvent Event interface
#[non_exhaustive]
#[derive(Debug)]
pub struct OfflineAudioCompletionEvent {
    /// The rendered AudioBuffer
    pub rendered_buffer: AudioBuffer,
    /// Inherits from this base Event
    pub event: Event,
}

#[derive(Debug)]
pub(crate) enum EventPayload {
    None,
    RenderCapacity(AudioRenderCapacityEvent),
    ProcessorError(ErrorEvent),
    #[cfg(feature = "diagnostics")]
    Diagnostics(AudioContextDiagnostics),
    Message(Box<dyn Any + Send + 'static>),
    AudioContextState(AudioContextState),
    Complete(AudioBuffer),
    AudioProcessing(AudioProcessingEvent),
}

#[derive(Debug)]
pub(crate) struct EventDispatch {
    type_: EventType,
    payload: EventPayload,
}

impl EventDispatch {
    pub fn ended(id: AudioNodeId) -> Self {
        EventDispatch {
            type_: EventType::Ended(id),
            payload: EventPayload::None,
        }
    }

    pub fn sink_change() -> Self {
        EventDispatch {
            type_: EventType::SinkChange,
            payload: EventPayload::None,
        }
    }

    pub fn state_change(state: AudioContextState) -> Self {
        EventDispatch {
            type_: EventType::StateChange,
            payload: EventPayload::AudioContextState(state),
        }
    }

    pub fn render_capacity(value: AudioRenderCapacityEvent) -> Self {
        EventDispatch {
            type_: EventType::RenderCapacity,
            payload: EventPayload::RenderCapacity(value),
        }
    }

    pub fn processor_error(id: AudioNodeId, value: ErrorEvent) -> Self {
        EventDispatch {
            type_: EventType::ProcessorError(id),
            payload: EventPayload::ProcessorError(value),
        }
    }

    #[cfg(feature = "diagnostics")]
    pub fn diagnostics(value: AudioContextDiagnostics) -> Self {
        EventDispatch {
            type_: EventType::Diagnostics,
            payload: EventPayload::Diagnostics(value),
        }
    }

    pub fn message(id: AudioNodeId, value: Box<dyn Any + Send + 'static>) -> Self {
        EventDispatch {
            type_: EventType::Message(id),
            payload: EventPayload::Message(value),
        }
    }

    pub fn complete(buffer: AudioBuffer) -> Self {
        EventDispatch {
            type_: EventType::Complete,
            payload: EventPayload::Complete(buffer),
        }
    }

    pub fn audio_processing(id: AudioNodeId, value: AudioProcessingEvent) -> Self {
        EventDispatch {
            type_: EventType::AudioProcessing(id),
            payload: EventPayload::AudioProcessing(value),
        }
    }
}

pub(crate) enum EventHandler {
    Once(Box<dyn FnOnce(EventPayload) + Send + 'static>),
    Multiple(Box<dyn FnMut(EventPayload) + Send + 'static>),
}

#[derive(Clone)]
pub(crate) struct EventLoop {
    event_recv: Receiver<EventDispatch>,
    event_handlers: Arc<Mutex<HashMap<EventType, EventHandler>>>,
}

impl EventLoop {
    pub fn new(event_recv: Receiver<EventDispatch>) -> Self {
        Self {
            event_recv,
            event_handlers: Default::default(),
        }
    }

    fn handle_event(&self, mut event: EventDispatch) -> ControlFlow<()> {
        // Terminate the event loop when the audio context is closing
        let mut result = ControlFlow::Continue(());
        if matches!(
            event.payload,
            EventPayload::AudioContextState(AudioContextState::Closed)
        ) {
            event.payload = EventPayload::None; // the statechange handler takes no argument
            result = ControlFlow::Break(());
        }

        // In unit tests, we rethrow panics to avoid missing critical node exceptions
        // https://github.com/orottier/web-audio-api-rs/issues/522
        #[cfg(test)]
        if let EventPayload::ProcessorError(e) = event.payload {
            panic!("Rethrowing exception during tests: {:?}", e);
        }

        let mut event_handler_lock = self.event_handlers.lock().unwrap();
        let callback_option = event_handler_lock.remove(&event.type_);
        drop(event_handler_lock); // release Mutex while running callback

        if let Some(callback) = callback_option {
            match callback {
                EventHandler::Once(f) => (f)(event.payload),
                EventHandler::Multiple(mut f) => {
                    (f)(event.payload);
                    self.event_handlers
                        .lock()
                        .unwrap()
                        .insert(event.type_, EventHandler::Multiple(f));
                }
            };
        }

        result
    }

    #[inline(always)]
    pub fn handle_pending_events(&self) -> bool {
        let mut events_were_handled = false;
        // try_iter will yield all pending events, but does not block
        for event in self.event_recv.try_iter() {
            // we can ignore the return value, it is only useful in the event thread
            let _ = self.handle_event(event);
            events_were_handled = true;
        }
        events_were_handled
    }

    pub fn run_in_thread(&self) {
        log::debug!("Entering event thread");

        // split borrows to help compiler
        let self_clone = self.clone();

        std::thread::spawn(move || {
            // This thread is dedicated to event handling, so we can block
            for event in self_clone.event_recv.iter() {
                let result = self_clone.handle_event(event);
                if result.is_break() {
                    break;
                }
            }

            log::debug!("Event loop has terminated");
        });
    }

    pub fn set_handler(&self, event: EventType, callback: EventHandler) {
        self.event_handlers.lock().unwrap().insert(event, callback);
    }

    pub fn clear_handler(&self, event: EventType) {
        self.event_handlers.lock().unwrap().remove(&event);
    }
}
