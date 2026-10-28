//! Renderer-independent animation playback and effect timing.
//!
//! The caller supplies a monotonic time for every operation. This crate does not
//! read the clock, schedule timers, own DOM nodes, parse CSS, or invoke JavaScript.
//! `AnimationEngine` manages stable identities and active instances. `sample`
//! is read-only; `command` controls playback; `advance` commits all active states
//! and returns a notification batch and the next sparse wake. The host applies
//! its parsed easing and typed interpolation, then dispatches any JavaScript.
//!
//! The default engine stores `AnimationState` directly. A host implementing the
//! two `PlaybackState` accessors can retain its associations in the same shared
//! allocation; no playback copy, browser dependency or callback is needed.
//!
//! ```
//! use moli_animation::{AnimationCommand, AnimationEngine, AnimationState};
//! use std::time::{Duration, Instant};
//!
//! let now = Instant::now();
//! let mut state = AnimationState::new(now);
//! state.has_effect = true;
//! state.duration = Duration::from_secs(1);
//! let mut engine = AnimationEngine::default();
//! let animation = engine.create(state);
//! let id = animation.borrow().id();
//! let outcome = engine.command(id, AnimationCommand::Play, now).unwrap();
//! assert_eq!(outcome.ready, moli_animation::ReadyPromiseUpdate::Pending);
//! let batch = engine.advance(now);
//! assert_eq!(batch.next_wake, Some(now + Duration::from_secs(1)));
//! let sample = engine.sample(id, now + Duration::from_millis(250)).unwrap();
//! assert_eq!(sample.progress.unwrap().fraction, 0.25);
//! ```
#![forbid(unsafe_code)]

mod clock;
mod engine;
mod keyframes;
mod playback;
mod timing;
mod transition;

pub use clock::AnimationClock;
pub use engine::{Advance, AnimationEngine, AnimationUpdate, PlaybackState};
pub use keyframes::resolve_keyframe_offsets;
pub use playback::{
    AdvanceMode, AnimationCommand, AnimationEvent, AnimationId, AnimationPhase, AnimationSample,
    AnimationState, CommandOutcome, EffectProgress, FinishedPromiseUpdate, InvalidPlaybackState,
    KeyframeValue, PhaseEvent, PlaybackNotification, PlaybackNotificationKind, PropertyKeyframes,
    PropertySample, ReadyPromiseUpdate, SamplingMode,
};
pub use timing::{CssTime, FillMode, active_duration, duration};
pub use transition::{ReversalTiming, reverse_transition_timing};
