//! DOM and CSS ownership around the renderer-independent playback state.
use moli_animation::{AnimationState, CssTime, EffectProgress, FillMode, PlaybackNotification};
use std::{
    cell::RefCell,
    rc::Rc,
    time::{Duration, Instant},
};

mod keyframes;
pub(crate) use keyframes::{
    KeyframeDeclarations, KeyframeTrackBuilder, SpecifiedKeyframes, SpecifiedSample,
};

pub(crate) type Animation = Rc<RefCell<AnimationInstance>>;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AnimationKind {
    CssAnimation,
    CssTransition,
    Script,
}

/// Browser cascade origin accompanies each engine value without coupling the
/// renderer-independent timing/effect model to CSS cascade levels.
pub(crate) struct EffectSample {
    pub(crate) value: SpecifiedSample,
    pub(crate) kind: AnimationKind,
}
impl std::ops::Deref for EffectSample {
    type Target = SpecifiedSample;
    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

/// setKeyframes takes over keyframes; replacing the effect also takes over all
/// effect timing. CSS play-state and the CSS association remain independent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CssEffectSource {
    Css,
    ScriptKeyframes,
    ScriptEffect,
}

#[derive(Debug, Default)]
pub(crate) struct TimingOverrides {
    pub(crate) delay: bool,
    pub(crate) duration: bool,
    pub(crate) iterations: bool,
    pub(crate) fill: bool,
    pub(crate) easing: bool,
    pub(crate) direction: bool,
}

/// An incoming CSS declaration. Script overrides are tracked per timing member,
/// so later CSS changes cannot undo effect.updateTiming().
#[derive(Debug)]
pub(crate) struct CssTiming {
    pub(crate) delay: CssTime,
    pub(crate) duration: Duration,
    pub(crate) iterations: f64,
    pub(crate) paused: bool,
    pub(crate) fill: FillMode,
    pub(crate) easing: String,
    pub(crate) direction: String,
}

/// Browser associations do not enter moli-animation or its dependency graph.
#[derive(Debug)]
pub(crate) struct AnimationInstance {
    pub(crate) playback: AnimationState,
    pub(crate) frames: Vec<SpecifiedKeyframes>,
    pub(crate) composite_order: u64,
    pub(crate) kind: AnimationKind,
    pub(crate) timeline_document: Option<crate::document_runtime::DomHandle>,
    pub(crate) execution_owner: Option<crate::native_bridge::WindowDocumentTaskTarget>,
    pub(crate) target: Option<crate::document_runtime::DomHandle>,
    pub(crate) registered_target: Option<crate::document_runtime::DomHandle>,
    pub(crate) css_owner: Option<crate::document_runtime::DomHandle>,
    pub(crate) name: String,
    pub(crate) easing: Easing,
    frame_easings: std::collections::HashMap<String, Easing>,
    pub(crate) retired: bool,
    pub(crate) published_finished_generation: Option<u64>,
    pub(crate) ignore_css_play_state: bool,
    pub(crate) timing_overrides: TimingOverrides,
    pub(crate) css_paused: Option<bool>,
    pub(crate) owned_by_css: bool,
    pub(crate) css_effect_source: CssEffectSource,
}
impl AnimationInstance {
    /// CSS control belongs to the browser association; playback and promise
    /// changes are decided once by the renderer-independent command contract.
    pub(crate) fn command(
        &mut self,
        command: moli_animation::AnimationCommand,
        now: Instant,
    ) -> Result<moli_animation::CommandOutcome, moli_animation::InvalidPlaybackState> {
        use moli_animation::AnimationCommand;
        let was_paused = self.playback.play_state(now) == "paused";
        let outcome = self.playback.command(command, now)?;
        let changed_paused = was_paused != (self.playback.play_state(now) == "paused");
        if matches!(
            command,
            AnimationCommand::Play | AnimationCommand::Pause | AnimationCommand::Cancel
        ) || (changed_paused
            && matches!(
                command,
                AnimationCommand::Reverse | AnimationCommand::SetStartTime { .. }
            ))
        {
            self.ignore_css_play_state = true;
        }
        Ok(outcome)
    }
    pub(crate) fn new(kind: AnimationKind, name: String, now: Instant) -> Self {
        let playback = if kind == AnimationKind::Script {
            AnimationState::new(now)
        } else {
            AnimationState::css(now)
        };
        Self {
            composite_order: playback.id().0,
            playback,
            frames: Vec::new(),
            kind,
            name,
            timeline_document: None,
            execution_owner: None,
            target: None,
            registered_target: None,
            css_owner: None,
            easing: Easing::linear(),
            frame_easings: Default::default(),
            retired: false,
            published_finished_generation: None,
            ignore_css_play_state: false,
            timing_overrides: TimingOverrides::default(),
            css_paused: None,
            owned_by_css: kind != AnimationKind::Script,
            css_effect_source: CssEffectSource::Css,
        }
    }
    pub(crate) fn copy_effect_from(&mut self, effect: &Self) {
        self.playback.copy_effect_from(&effect.playback);
        self.frames = effect.frames.clone();
        self.easing = effect.easing.clone();
        self.frame_easings = effect.frame_easings.clone();
        self.target = effect.target;
    }
    pub(crate) fn detached_effect(&self) -> Animation {
        let mut effect = Self::new(AnimationKind::Script, String::new(), Instant::now());
        effect.copy_effect_from(self);
        effect.shared()
    }
    pub(crate) fn shared(self) -> Animation {
        Rc::new(RefCell::new(self))
    }
    pub(crate) fn cancel(&mut self, now: Instant) {
        self.ignore_css_play_state = true;
        self.playback.cancel(now);
    }
    pub(crate) fn progress(&self, now: Instant) -> Option<f64> {
        let EffectProgress { fraction, before } = self.playback.progress(now)?;
        Some(self.easing.sample(fraction, before))
    }
    pub(crate) fn set_keyframes(&mut self, mut frames: Vec<SpecifiedKeyframes>) {
        for frame in &mut frames {
            frame.ensure_neutral_endpoints();
        }
        self.frame_easings = frames
            .iter()
            .flat_map(|f| &f.values)
            .filter_map(|v| v.easing.as_ref())
            .filter_map(|css| Easing::parse(css).map(|e| (css.clone(), e)))
            .collect();
        self.frames = frames;
    }
    pub(crate) fn sample_interval(&self, property: &str, now: Instant) -> Option<SpecifiedSample> {
        self.sample_interval_for(property, now, moli_animation::SamplingMode::Presentation)
    }
    pub(crate) fn sample_interval_for(
        &self,
        property: &str,
        now: Instant,
        mode: moli_animation::SamplingMode,
    ) -> Option<SpecifiedSample> {
        let progress = self.playback.progress_for_sample(now, mode)?;
        let fraction = self.easing.sample(progress.fraction, progress.before);
        let mut sample = self
            .frames
            .iter()
            .rev()
            .find(|frame| frame.property == property)?
            .sample_interval(fraction)?;
        if let Some(css) = sample.easing.take()
            && let Some(easing) = self.frame_easings.get(&css)
        {
            sample.progress = easing.sample(sample.progress, progress.before);
        }
        Some(sample)
    }
    pub(crate) fn update_css_timing(&mut self, next: CssTiming, changed_at: Instant) {
        self.playback
            .preserve_current_time_on_timing_change(changed_at);
        if self.css_effect_source != CssEffectSource::ScriptEffect {
            macro_rules! update {
                ($field:ident) => {
                    if !self.timing_overrides.$field {
                        self.playback.$field = next.$field;
                    }
                };
            }
            update!(delay);
            update!(duration);
            update!(iterations);
            update!(fill);
            if !self.timing_overrides.direction {
                self.playback.direction = next.direction;
            }
            // CSS animation easing is attached to each keyframe interval. Effect
            // easing remains linear unless explicitly overridden through WAAPI.
            if self.kind != AnimationKind::CssAnimation
                && !self.timing_overrides.easing
                && self.easing.css != next.easing
            {
                self.easing = Easing::parse(&next.easing).unwrap_or_else(Easing::linear);
            }
        }
        if !self.ignore_css_play_state && self.css_paused != Some(next.paused) {
            self.playback.set_css_paused(next.paused, changed_at);
        }
        self.css_paused = Some(next.paused);
    }
}

impl moli_animation::PlaybackState for AnimationInstance {
    fn playback(&self) -> &AnimationState {
        &self.playback
    }
    fn playback_mut(&mut self) -> &mut AnimationState {
        &mut self.playback
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Easing {
    pub(crate) css: String,
    function: style::values::computed::easing::TimingFunction,
}

impl Easing {
    pub(crate) fn parse(value: &str) -> Option<Self> {
        use style::{
            context::QuirksMode,
            custom_properties::AttrTaint,
            parser::{Parse, ParserContext},
            stylesheets::{CssRuleType, Origin, UrlExtraData},
        };
        let url = UrlExtraData::from(url::Url::parse("about:blank").ok()?);
        let context = ParserContext::new(
            Origin::Author,
            &url,
            Some(CssRuleType::Style),
            style_traits::ParsingMode::DEFAULT,
            QuirksMode::NoQuirks,
            Default::default(),
            None,
            None,
            AttrTaint::default(),
        );
        let function = cssparser::Parser::new(value)
            .parse_entirely(|input| {
                style::values::specified::easing::TimingFunction::parse(&context, input)
            })
            .ok()?
            .to_computed_value_without_context()?;
        Some(Self {
            css: value.to_owned(),
            function,
        })
    }
    pub(crate) fn linear() -> Self {
        Self {
            css: "linear".to_owned(),
            function: style::values::computed::easing::TimingFunction::Keyword(
                style::values::generics::easing::TimingKeyword::Linear,
            ),
        }
    }
    pub(crate) fn sample(&self, progress: f64, before: bool) -> f64 {
        use style::values::generics::easing::BeforeFlag;
        self.function.calculate_output(
            progress,
            if before {
                BeforeFlag::Set
            } else {
                BeforeFlag::Unset
            },
            1e-7,
        )
    }
}

/// Notifications are values collected during commit and delivered afterwards.
pub(crate) enum CssLifecycleEvent {
    Animation(&'static str, String, f64),
    Transition(&'static str, String, f64),
    Playback(PlaybackEvent),
}

pub(crate) struct PlaybackEvent {
    pub(crate) animation: Animation,
    pub(crate) notification: PlaybackNotification,
    pub(crate) queued: bool,
}
impl CssLifecycleEvent {
    pub(crate) fn playback(animation: Animation, notification: PlaybackNotification) -> Self {
        Self::Playback(PlaybackEvent {
            animation,
            queued: notification.event_is_queued(),
            notification,
        })
    }
}
