use super::AudioNode;
use crate::events::{Event, EventHandler, EventType};

/// Interface of source nodes, controlling start and stop times.
/// The node will emit silence before it is started, and after it has ended.
pub trait AudioScheduledSourceNode: AudioNode {
    /// Play immediately
    ///
    /// # Panics
    ///
    /// Panics if the source was already started
    fn start(&mut self);

    /// Schedule playback start at given timestamp
    ///
    /// # Panics
    ///
    /// Panics if the source was already started
    fn start_at(&mut self, when: f64);

    /// Stop immediately
    ///
    /// # Panics
    ///
    /// Panics if the source was not started yet
    fn stop(&mut self);

    /// Schedule playback stop at given timestamp
    ///
    /// # Panics
    ///
    /// Panics if the source was not started yet
    fn stop_at(&mut self, when: f64);

    /// Register callback to run when the source node has stopped playing
    ///
    /// For all [`AudioScheduledSourceNode`]s, the ended event is dispatched when the stop time
    /// determined by stop() is reached. For an
    /// [`AudioBufferSourceNode`](crate::node::AudioBufferSourceNode), the event is also dispatched
    /// because the duration has been reached or if the entire buffer has been played.
    ///
    /// Only a single event handler is active at any time. Calling this method multiple times will
    /// override the previous event handler.
    fn set_onended<F: FnOnce(Event) + Send + 'static>(&self, callback: F) {
        let callback = move |_| callback(Event { type_: "ended" });

        self.context().set_event_handler(
            EventType::Ended(self.registration().id()),
            EventHandler::Once(Box::new(callback)),
        );
    }

    /// Unset the callback to run when the source node has stopped playing
    fn clear_onended(&self) {
        self.context()
            .clear_event_handler(EventType::Ended(self.registration().id()));
    }
}

#[cfg(test)]
mod tests {
    use crate::context::{AudioContextRegistration, BaseAudioContext, OfflineAudioContext};
    use crate::node::{AudioNode, AudioScheduledSourceNode, ChannelConfig};

    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;

    enum ConcreteAudioScheduledSourceNode {
        Buffer(crate::node::AudioBufferSourceNode),
        Constant(crate::node::ConstantSourceNode),
        Oscillator(crate::node::OscillatorNode),
    }
    use ConcreteAudioScheduledSourceNode::*;

    impl AudioNode for ConcreteAudioScheduledSourceNode {
        fn registration(&self) -> &AudioContextRegistration {
            match self {
                Buffer(n) => n.registration(),
                Constant(n) => n.registration(),
                Oscillator(n) => n.registration(),
            }
        }

        fn channel_config(&self) -> &ChannelConfig {
            match self {
                Buffer(n) => n.channel_config(),
                Constant(n) => n.channel_config(),
                Oscillator(n) => n.channel_config(),
            }
        }

        fn number_of_inputs(&self) -> usize {
            match self {
                Buffer(n) => n.number_of_inputs(),
                Constant(n) => n.number_of_inputs(),
                Oscillator(n) => n.number_of_inputs(),
            }
        }

        fn number_of_outputs(&self) -> usize {
            match self {
                Buffer(n) => n.number_of_outputs(),
                Constant(n) => n.number_of_outputs(),
                Oscillator(n) => n.number_of_outputs(),
            }
        }
    }

    impl AudioScheduledSourceNode for ConcreteAudioScheduledSourceNode {
        fn start(&mut self) {
            match self {
                Buffer(n) => n.start(),
                Constant(n) => n.start(),
                Oscillator(n) => n.start(),
            }
        }

        fn start_at(&mut self, when: f64) {
            match self {
                Buffer(n) => n.start_at(when),
                Constant(n) => n.start_at(when),
                Oscillator(n) => n.start_at(when),
            }
        }

        fn stop(&mut self) {
            match self {
                Buffer(n) => n.stop(),
                Constant(n) => n.stop(),
                Oscillator(n) => n.stop(),
            }
        }

        fn stop_at(&mut self, when: f64) {
            match self {
                Buffer(n) => n.stop_at(when),
                Constant(n) => n.stop_at(when),
                Oscillator(n) => n.stop_at(when),
            }
        }
    }

    fn run_ended_event(f: impl FnOnce(&OfflineAudioContext) -> ConcreteAudioScheduledSourceNode) {
        let mut context = OfflineAudioContext::new(2, 44_100, 44_100.);
        let mut src = f(&context);
        src.start_at(0.);
        src.stop_at(0.5);

        let ended = Arc::new(AtomicBool::new(false));
        let ended_clone = Arc::clone(&ended);
        src.set_onended(move |_event| {
            ended_clone.store(true, Ordering::Relaxed);
        });

        let _ = context.start_rendering_sync();
        assert!(ended.load(Ordering::Relaxed));
    }

    #[test]
    fn test_ended_event_constant_source() {
        run_ended_event(|c| Constant(c.create_constant_source()));
    }
    #[test]
    fn test_ended_event_buffer_source() {
        run_ended_event(|c| Buffer(c.create_buffer_source()));
    }
    #[test]
    fn test_ended_event_oscillator() {
        run_ended_event(|c| Oscillator(c.create_oscillator()));
    }

    fn run_no_ended_event(
        f: impl FnOnce(&OfflineAudioContext) -> ConcreteAudioScheduledSourceNode,
    ) {
        let mut context = OfflineAudioContext::new(2, 44_100, 44_100.);
        let src = f(&context);

        // do not start the node

        let ended = Arc::new(AtomicBool::new(false));
        let ended_clone = Arc::clone(&ended);
        src.set_onended(move |_event| {
            ended_clone.store(true, Ordering::Relaxed);
        });

        let _ = context.start_rendering_sync();
        assert!(!ended.load(Ordering::Relaxed)); // should not have triggered
    }

    #[test]
    fn test_no_ended_event_constant_source() {
        run_no_ended_event(|c| Constant(c.create_constant_source()));
    }
    #[test]
    fn test_no_ended_event_buffer_source() {
        run_no_ended_event(|c| Buffer(c.create_buffer_source()));
    }
    #[test]
    fn test_no_ended_event_oscillator() {
        run_no_ended_event(|c| Oscillator(c.create_oscillator()));
    }

    fn run_exact_ended_event(
        f: impl FnOnce(&OfflineAudioContext) -> ConcreteAudioScheduledSourceNode,
    ) {
        let mut context = OfflineAudioContext::new(2, 44_100, 44_100.);
        let mut src = f(&context);
        src.start_at(0.);
        src.stop_at(1.); // end right at the end of the offline buffer

        let ended = Arc::new(AtomicBool::new(false));
        let ended_clone = Arc::clone(&ended);
        src.set_onended(move |_event| {
            ended_clone.store(true, Ordering::Relaxed);
        });

        let _ = context.start_rendering_sync();
        assert!(ended.load(Ordering::Relaxed));
    }

    #[test]
    fn test_exact_ended_event_constant_source() {
        run_exact_ended_event(|c| Constant(c.create_constant_source()));
    }
    #[test]
    fn test_exact_ended_event_buffer_source() {
        run_exact_ended_event(|c| Buffer(c.create_buffer_source()));
    }
    #[test]
    fn test_exact_ended_event_oscillator() {
        run_exact_ended_event(|c| Oscillator(c.create_oscillator()));
    }

    fn run_implicit_ended_event(
        f: impl FnOnce(&OfflineAudioContext) -> ConcreteAudioScheduledSourceNode,
    ) {
        let mut context = OfflineAudioContext::new(2, 44_100, 44_100.);
        let mut src = f(&context);
        src.start_at(0.);
        // no explicit stop, so we stop at end of offline context

        let ended = Arc::new(AtomicBool::new(false));
        let ended_clone = Arc::clone(&ended);
        src.set_onended(move |_event| {
            ended_clone.store(true, Ordering::Relaxed);
        });

        let _ = context.start_rendering_sync();
        assert!(ended.load(Ordering::Relaxed));
    }

    #[test]
    fn test_implicit_ended_event_constant_source() {
        run_implicit_ended_event(|c| Constant(c.create_constant_source()));
    }

    #[test]
    fn test_implicit_ended_event_buffer_source() {
        run_implicit_ended_event(|c| Buffer(c.create_buffer_source()));
    }

    #[test]
    fn test_implicit_ended_event_oscillator() {
        run_implicit_ended_event(|c| Oscillator(c.create_oscillator()));
    }

    fn run_start_twice(f: impl FnOnce(&OfflineAudioContext) -> ConcreteAudioScheduledSourceNode) {
        let context = OfflineAudioContext::new(2, 1, 44_100.);
        let mut src = f(&context);
        src.start();
        src.start();
    }

    #[test]
    #[should_panic]
    fn test_start_twice_constant_source() {
        run_start_twice(|c| Constant(c.create_constant_source()));
    }

    #[test]
    #[should_panic]
    fn test_start_twice_buffer_source() {
        run_start_twice(|c| Buffer(c.create_buffer_source()));
    }

    #[test]
    #[should_panic]
    fn test_start_twice_oscillator() {
        run_start_twice(|c| Oscillator(c.create_oscillator()));
    }

    fn run_stop_before_start(
        f: impl FnOnce(&OfflineAudioContext) -> ConcreteAudioScheduledSourceNode,
    ) {
        let context = OfflineAudioContext::new(2, 1, 44_100.);
        let mut src = f(&context);
        src.stop();
    }

    #[test]
    #[should_panic]
    fn test_stop_before_start_constant_source() {
        run_stop_before_start(|c| Constant(c.create_constant_source()));
    }

    #[test]
    #[should_panic]
    fn test_stop_before_start_buffer_source() {
        run_stop_before_start(|c| Buffer(c.create_buffer_source()));
    }

    #[test]
    #[should_panic]
    fn test_stop_before_start_oscillator() {
        run_stop_before_start(|c| Oscillator(c.create_oscillator()));
    }

    fn run_stop_twice(f: impl FnOnce(&OfflineAudioContext) -> ConcreteAudioScheduledSourceNode) {
        // is allowed, see https://github.com/orottier/web-audio-api-rs/issues/579
        let context = OfflineAudioContext::new(2, 1, 44_100.);
        let mut src = f(&context);
        src.start();
        src.stop();
        src.stop();
    }

    #[test]
    fn test_stop_twice_allowed_constant_source() {
        run_stop_twice(|c| Constant(c.create_constant_source()));
    }
    #[test]
    fn test_stop_twice_allowed_buffer_source() {
        run_stop_twice(|c| Buffer(c.create_buffer_source()));
    }
    #[test]
    fn test_stop_twice_allowed_oscillator() {
        run_stop_twice(|c| Oscillator(c.create_oscillator()));
    }
}
