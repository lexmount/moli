use std::time::{Duration, Instant};

use style::{
    animation::{Animation, AnimationState, ElementAnimationSet, KeyframesIterationState},
    invalidation::element::restyle_hints::RestyleHint,
    values::computed::{AnimationDirection, AnimationFillMode},
};

use crate::{document_runtime::DomHandle, dom::native::DomHost};

use super::{MoliStyleEngine, StyleInvalidationScope};

impl MoliStyleEngine {
    pub(crate) fn set_screen_animations_enabled(&self, enabled: bool) {
        self.document_worlds
            .animation_time
            .set(enabled.then(|| self.document_worlds.animation_epoch.elapsed().as_secs_f64()));
        if !enabled {
            for world in self.document_worlds.worlds.borrow().values() {
                world.animations.sets.write().clear();
            }
        }
    }

    /// Earliest time an animation needs a new style sample. During a positive
    /// delay the cascade is constant, so the page can wait until the start.
    pub(crate) fn next_screen_animation_delay(
        &self,
        host: &DomHost,
        now: Instant,
    ) -> Option<Duration> {
        let sampled_at = self.document_worlds.animation_time.get()?;
        let mut next: Option<f64> = None;
        for world in self.document_worlds.worlds.borrow().values() {
            for (key, set) in world.animations.sets.read().iter() {
                if !is_connected_to_document(host, DomHandle::new(key.node.0), world.document) {
                    continue;
                }
                let animation_times = set.animations.iter().filter_map(|animation| match animation
                    .state
                {
                    AnimationState::Pending => Some(animation.started_at),
                    AnimationState::Running if !animation.has_ended(sampled_at) => Some(sampled_at),
                    _ => None,
                });
                let transition_times =
                    set.transitions
                        .iter()
                        .filter_map(|transition| match transition.state {
                            AnimationState::Pending => Some(transition.start_time),
                            AnimationState::Running => Some(sampled_at),
                            _ => None,
                        });
                for time in animation_times.chain(transition_times) {
                    next = Some(next.map_or(time, |previous| previous.min(time)));
                }
            }
        }
        let time = next?;
        let now = now
            .saturating_duration_since(self.document_worlds.animation_epoch)
            .as_secs_f64();
        Duration::try_from_secs_f64((time - now).max(0.0)).ok()
    }

    pub(crate) fn sample_screen_animations(&self, host: &DomHost, now: Instant) -> bool {
        let clock = &self.document_worlds.animation_time;
        let Some(previous) = clock.get() else {
            return false;
        };
        let now = now
            .saturating_duration_since(self.document_worlds.animation_epoch)
            .as_secs_f64();
        if now <= previous {
            return false;
        }
        clock.set(Some(now));
        let mut changed = false;
        for world in self.document_worlds.worlds.borrow().values() {
            let mut targets = Vec::new();
            world.animations.sets.write().retain(|key, set| {
                let handle = DomHandle::new(key.node.0);
                if !is_connected_to_document(host, handle, world.document) {
                    return false;
                }
                if sample_animation_set(set, previous, now) {
                    self.dom_adapter
                        .mark_element_style_dirty(handle, RestyleHint::RESTYLE_SELF);
                    targets.push((handle, StyleInvalidationScope::Element));
                }
                true
            });
            if !targets.is_empty() {
                changed = true;
                world.document_state.note_screen_style_change();
                world.document_state.record_style_targets(
                    host,
                    &self.dom_adapter,
                    world.document,
                    targets,
                );
            }
        }
        changed
    }
}

fn is_connected_to_document(host: &DomHost, handle: DomHandle, document: DomHandle) -> bool {
    host.is_connected(handle) && host.owner_document_handle(handle) == Some(document)
}

fn sample_animation_set(set: &mut ElementAnimationSet, previous: f64, now: f64) -> bool {
    let mut changed = false;
    for animation in &mut set.animations {
        match animation.state {
            AnimationState::Pending if now >= animation.started_at => {
                animation.state = AnimationState::Running;
            }
            AnimationState::Running if !animation.has_ended(previous) => {}
            _ => continue,
        }
        advance_iterations(animation, now);
        if animation.has_ended(now)
            && !matches!(
                animation.fill_mode,
                AnimationFillMode::Forwards | AnimationFillMode::Both
            )
        {
            animation.state = AnimationState::Finished;
        }
        changed = true;
    }
    for transition in &mut set.transitions {
        match transition.state {
            AnimationState::Pending if now >= transition.start_time => {
                transition.state = AnimationState::Running;
            }
            AnimationState::Running => {}
            _ => continue,
        }
        if transition.has_ended(now) {
            transition.state = AnimationState::Finished;
        }
        changed = true;
    }
    changed
}

fn advance_iterations(animation: &mut Animation, now: f64) {
    // A background tab can skip many iterations. Retain only the current
    // iteration, and leave a finite animation on its last iteration so its
    // fill value can still be sampled.
    let elapsed = ((now - animation.started_at) / animation.duration).floor();
    let count = match &mut animation.iteration_state {
        KeyframesIterationState::Finite(current, limit) => {
            let count = elapsed.min((*limit - *current).ceil() - 1.0).max(0.0);
            *current += count;
            count
        }
        KeyframesIterationState::Infinite(current) => {
            *current += elapsed;
            elapsed
        }
    };
    animation.started_at += count * animation.duration;
    if count % 2.0 == 1.0
        && matches!(
            animation.direction,
            AnimationDirection::Alternate | AnimationDirection::AlternateReverse
        )
    {
        animation.current_direction = if animation.current_direction == AnimationDirection::Normal {
            AnimationDirection::Reverse
        } else {
            AnimationDirection::Normal
        };
    }
}
