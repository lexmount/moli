use crossbeam_channel::Sender;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::context::{BaseAudioContext, ConcreteBaseAudioContext};
use crate::events::{EventDispatch, EventHandler, EventPayload, EventType};
use crate::stats::{AudioStats, AudioStatsSnapshot};
use crate::Event;

/// Options for constructing an `AudioRenderCapacity`
#[derive(Clone, Debug)]
pub struct AudioRenderCapacityOptions {
    /// An update interval (in seconds) for dispatching [`AudioRenderCapacityEvent`]s
    pub update_interval: f64,
}

impl Default for AudioRenderCapacityOptions {
    fn default() -> Self {
        Self {
            update_interval: 1.,
        }
    }
}

/// Performance metrics of the rendering thread
#[derive(Clone, Debug)]
pub struct AudioRenderCapacityEvent {
    /// The start time of the data collection period in terms of the associated AudioContext's currentTime
    pub timestamp: f64,
    /// An average of collected load values over the given update interval
    pub average_load: f64,
    /// A maximum value from collected load values over the given update interval.
    pub peak_load: f64,
    /// A ratio between the number of buffer underruns and the total number of system-level audio callbacks over the given update interval.
    pub underrun_ratio: f64,
    /// Inherits from this base Event
    pub event: Event,
}

impl AudioRenderCapacityEvent {
    fn new(timestamp: f64, average_load: f64, peak_load: f64, underrun_ratio: f64) -> Self {
        // We are limiting the precision here conform
        // https://webaudio.github.io/web-audio-api/#dom-audiorendercapacityevent-averageload
        Self {
            timestamp,
            average_load: (average_load * 100.).round() / 100.,
            peak_load: (peak_load * 100.).round() / 100.,
            underrun_ratio: (underrun_ratio * 100.).ceil() / 100.,
            event: Event {
                type_: "AudioRenderCapacityEvent",
            },
        }
    }
}

/// Provider for rendering performance metrics
///
/// A load value is computed for each system-level audio callback, by dividing its execution
/// duration by the system-level audio callback buffer size divided by the sample rate.
///
/// Ideally the load value is below 1.0, meaning that it took less time to render the audio than it
/// took to play it out. An audio buffer underrun happens when this load value is greater than 1.0: the
/// system could not render audio fast enough for real-time.
#[derive(Clone)]
pub struct AudioRenderCapacity {
    context: ConcreteBaseAudioContext,
    stats: AudioStats,
    stop_send: Arc<Mutex<Option<Sender<()>>>>,
}

impl std::fmt::Debug for AudioRenderCapacity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AudioRenderCapacity")
            .field(
                "context",
                &format!("BaseAudioContext@{}", self.context.address()),
            )
            .finish_non_exhaustive()
    }
}

impl AudioRenderCapacity {
    pub(crate) fn new(context: ConcreteBaseAudioContext, stats: AudioStats) -> Self {
        let stop_send = Arc::new(Mutex::new(None));

        Self {
            context,
            stats,
            stop_send,
        }
    }

    /// Start metric collection and analysis
    #[allow(clippy::missing_panics_doc)]
    pub fn start(&self, options: AudioRenderCapacityOptions) {
        // stop current metric collection, if any
        self.stop();

        let (stop_send, stop_recv) = crossbeam_channel::bounded(0);
        *self.stop_send.lock().unwrap() = Some(stop_send);

        let mut timestamp: f64 = self.context.current_time();
        let update_interval = Duration::from_secs_f64(options.update_interval.max(0.001));
        let base_context = self.context.clone();
        let stats = self.stats.clone();
        let mut previous = stats.snapshot();
        stats.take_peak_load();
        std::thread::spawn(move || loop {
            if stop_recv.recv_timeout(update_interval).is_ok() {
                return;
            }

            let next = stats.snapshot();
            if next.callback_count == previous.callback_count {
                continue;
            }

            let peak_load = stats.take_peak_load();
            let event = render_capacity_event(timestamp, previous, next, peak_load);

            let send_result = base_context.send_event(EventDispatch::render_capacity(event));
            if send_result.is_err() {
                break;
            };

            previous = next;
            timestamp = base_context.current_time();
        });
    }

    /// Stop metric collection and analysis
    #[allow(clippy::missing_panics_doc)]
    pub fn stop(&self) {
        // halt callback thread
        if let Some(stop_send) = self.stop_send.lock().unwrap().take() {
            let _ = stop_send.send(());
        }
    }

    /// The EventHandler for [`AudioRenderCapacityEvent`].
    ///
    /// Only a single event handler is active at any time. Calling this method multiple times will
    /// override the previous event handler.
    pub fn set_onupdate<F: FnMut(AudioRenderCapacityEvent) + Send + 'static>(
        &self,
        mut callback: F,
    ) {
        let callback = move |v| match v {
            EventPayload::RenderCapacity(v) => callback(v),
            _ => unreachable!(),
        };

        self.context.set_event_handler(
            EventType::RenderCapacity,
            EventHandler::Multiple(Box::new(callback)),
        );
    }

    /// Unset the EventHandler for [`AudioRenderCapacityEvent`].
    pub fn clear_onupdate(&self) {
        self.context.clear_event_handler(EventType::RenderCapacity);
    }
}

fn render_capacity_event(
    timestamp: f64,
    previous: AudioStatsSnapshot,
    next: AudioStatsSnapshot,
    peak_load: f64,
) -> AudioRenderCapacityEvent {
    let callback_count = next
        .callback_count
        .saturating_sub(previous.callback_count)
        .max(1);
    let render_duration = next
        .render_duration_ns_total
        .saturating_sub(previous.render_duration_ns_total);
    let callback_budget = next
        .callback_budget_ns_total
        .saturating_sub(previous.callback_budget_ns_total);
    let underruns = next.underrun_count.saturating_sub(previous.underrun_count);

    let average_load = if callback_budget == 0 {
        0.
    } else {
        render_duration as f64 / callback_budget as f64
    };

    AudioRenderCapacityEvent::new(
        timestamp,
        average_load,
        peak_load,
        underruns as f64 / callback_count as f64,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::context::{AudioContext, AudioContextOptions};

    #[test]
    fn test_same_instance() {
        let options = AudioContextOptions {
            sink_id: "none".into(),
            ..AudioContextOptions::default()
        };
        let context = AudioContext::new(options);

        let rc1 = context.render_capacity();
        let rc2 = context.render_capacity();
        let rc3 = rc2.clone();

        // assert all items are actually the same instance
        assert!(Arc::ptr_eq(&rc1.stop_send, &rc2.stop_send));
        assert!(Arc::ptr_eq(&rc1.stop_send, &rc3.stop_send));
    }

    #[test]
    fn test_stop_when_not_running() {
        let options = AudioContextOptions {
            sink_id: "none".into(),
            ..AudioContextOptions::default()
        };
        let context = AudioContext::new(options);

        let rc = context.render_capacity();
        rc.stop();
    }

    #[test]
    fn test_render_capacity() {
        let options = AudioContextOptions {
            sink_id: "none".into(),
            ..AudioContextOptions::default()
        };
        let context = AudioContext::new(options);

        let rc = context.render_capacity();
        let (send, recv) = crossbeam_channel::bounded(1);
        rc.set_onupdate(move |e| send.send(e).unwrap());
        rc.start(AudioRenderCapacityOptions {
            update_interval: 0.05,
        });
        let event = recv.recv().unwrap();

        assert!(event.timestamp >= 0.);
        assert!(event.average_load >= 0.);
        assert!(event.peak_load >= 0.);
        assert!(event.underrun_ratio >= 0.);

        assert!(event.timestamp.is_finite());
        assert!(event.average_load.is_finite());
        assert!(event.peak_load.is_finite());
        assert!(event.underrun_ratio.is_finite());

        assert_eq!(event.event.type_, "AudioRenderCapacityEvent");
    }

    #[test]
    fn test_render_capacity_stops_on_close() {
        let options = AudioContextOptions {
            sink_id: "none".into(),
            ..AudioContextOptions::default()
        };
        let context = AudioContext::new(options);

        let rc = context.render_capacity();
        let (send, recv) = crossbeam_channel::unbounded();
        rc.set_onupdate(move |e| send.send(e).unwrap());
        rc.start(AudioRenderCapacityOptions {
            update_interval: 0.01,
        });

        recv.recv().unwrap();
        while recv.try_recv().is_ok() {}

        context.close_sync();
        std::thread::sleep(std::time::Duration::from_millis(100));

        assert_eq!(recv.try_iter().count(), 0);
    }
}
