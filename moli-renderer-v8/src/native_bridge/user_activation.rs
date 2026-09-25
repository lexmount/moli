use std::{
    cell::Cell,
    time::{Duration, Instant},
};

/// Activation belongs to a Window, and can outlive its browsing context when
/// script retains that Window's Navigator or UserActivation object.
#[derive(Default)]
pub(crate) struct WindowUserActivationState {
    last_activation: Cell<Option<Instant>>,
    consumed: Cell<bool>,
}

impl WindowUserActivationState {
    pub(super) fn notify(&self) {
        self.last_activation.set(Some(Instant::now()));
        self.consumed.set(false);
    }

    pub(super) fn consume(&self) {
        self.consumed.set(true);
    }

    pub(crate) fn state(&self) -> (bool, bool) {
        let activation = self.last_activation.get();
        (
            activation.is_some_and(|time| time.elapsed() < Duration::from_secs(5))
                && !self.consumed.get(),
            activation.is_some(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expired_activation_keeps_sticky_state() {
        let state = WindowUserActivationState::default();
        assert_eq!(state.state(), (false, false));
        state.notify();
        assert_eq!(state.state(), (true, true));
        state
            .last_activation
            .set(Some(Instant::now() - Duration::from_secs(6)));
        assert_eq!(state.state(), (false, true));
        state.notify();
        assert_eq!(state.state(), (true, true));
    }

    #[test]
    fn consumption_preserves_sticky_state_until_the_next_activation() {
        let state = WindowUserActivationState::default();
        state.notify();
        state.consume();
        assert_eq!(state.state(), (false, true));
        state.notify();
        assert_eq!(state.state(), (true, true));
    }
}
