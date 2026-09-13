use std::{
    cell::Cell,
    time::{Duration, Instant},
};

/// Activation belongs to a Window, and can outlive its browsing context when
/// script retains that Window's Navigator or UserActivation object.
#[derive(Default)]
pub(crate) struct WindowUserActivationState {
    last_activation: Cell<Option<Instant>>,
}

impl WindowUserActivationState {
    pub(super) fn notify(&self) {
        self.last_activation.set(Some(Instant::now()));
    }

    pub(crate) fn state(&self) -> (bool, bool) {
        let activation = self.last_activation.get();
        (
            activation.is_some_and(|time| time.elapsed() < Duration::from_secs(5)),
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
}
