use std::{cell::RefCell, rc::Rc};

use super::JsContextHost;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct InputModifiers(u8);

impl InputModifiers {
    const ALT: u8 = 1;
    const CONTROL: u8 = 2;
    const META: u8 = 4;
    const SHIFT: u8 = 8;

    const fn from_bits(bits: u8) -> Self {
        Self(bits)
    }

    const fn alt(self) -> bool {
        self.0 & Self::ALT != 0
    }

    const fn control(self) -> bool {
        self.0 & Self::CONTROL != 0
    }

    const fn meta(self) -> bool {
        self.0 & Self::META != 0
    }

    const fn shift(self) -> bool {
        self.0 & Self::SHIFT != 0
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CurrentInputEventKind {
    MouseUp { button: i32 },
    Mouse,
    EnterKey,
    Other,
}

/// The native default-action context of the real platform input currently
/// being handled. DOM events are deliberately not stored here: synthetic events may
/// describe modifiers, but cannot manufacture the corresponding user intent.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct CurrentInputEvent {
    kind: CurrentInputEventKind,
    modifiers: InputModifiers,
}

impl CurrentInputEvent {
    pub(crate) fn mouse(event_name: &str, button: i32, modifiers: u8) -> Self {
        Self {
            kind: if event_name == "mouseup" {
                CurrentInputEventKind::MouseUp { button }
            } else {
                CurrentInputEventKind::Mouse
            },
            modifiers: InputModifiers::from_bits(modifiers),
        }
    }

    pub(crate) fn keyboard(key: &str, modifiers: u8) -> Self {
        Self {
            kind: if key.eq_ignore_ascii_case("enter") {
                CurrentInputEventKind::EnterKey
            } else {
                CurrentInputEventKind::Other
            },
            modifiers: InputModifiers::from_bits(modifiers),
        }
    }

    pub(crate) fn navigation_policy(self) -> InputNavigationPolicy {
        let button = match self.kind {
            CurrentInputEventKind::MouseUp { button } => button,
            CurrentInputEventKind::EnterKey => 0,
            CurrentInputEventKind::Mouse | CurrentInputEventKind::Other => {
                return InputNavigationPolicy::Current;
            }
        };
        navigation_policy_from_modifiers(button, self.modifiers)
    }
}

/// Chromium-shaped navigation intent before Moli folds window and tab chrome
/// into its foreground/background surface model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InputNavigationPolicy {
    Current,
    Download,
    NewWindow,
    NewForegroundSurface,
    NewBackgroundSurface,
}

/// Computes a DOM event's requested policy, then applies Blink's synthetic
/// download and tab-under protections against the ambient real input.
pub(crate) fn navigation_policy_from_event(
    button: i32,
    modifiers: u8,
    current_input: Option<CurrentInputEvent>,
) -> InputNavigationPolicy {
    let event_policy =
        navigation_policy_from_modifiers(button, InputModifiers::from_bits(modifiers));
    let input_policy = current_input
        .map(CurrentInputEvent::navigation_policy)
        .unwrap_or(InputNavigationPolicy::Current);

    match event_policy {
        InputNavigationPolicy::Download if input_policy != InputNavigationPolicy::Download => {
            InputNavigationPolicy::Current
        }
        InputNavigationPolicy::NewBackgroundSurface
            if input_policy != InputNavigationPolicy::NewBackgroundSurface =>
        {
            InputNavigationPolicy::NewForegroundSurface
        }
        _ => event_policy,
    }
}

fn navigation_policy_from_modifiers(
    button: i32,
    modifiers: InputModifiers,
) -> InputNavigationPolicy {
    let platform_new_tab_modifier = if cfg!(target_os = "macos") {
        modifiers.meta()
    } else {
        modifiers.control()
    };

    let requests_new_tab = button == 1 || platform_new_tab_modifier;
    if !requests_new_tab && !modifiers.shift() && !modifiers.alt() {
        InputNavigationPolicy::Current
    } else if requests_new_tab {
        if modifiers.shift() {
            InputNavigationPolicy::NewForegroundSurface
        } else {
            InputNavigationPolicy::NewBackgroundSurface
        }
    } else if modifiers.shift() {
        InputNavigationPolicy::NewWindow
    } else {
        InputNavigationPolicy::Download
    }
}

/// Restores the ambient real input automatically across early returns,
/// nested input dispatch, and unwinding.
#[must_use = "dropping the scope immediately would discard the current input event"]
pub(crate) struct CurrentInputEventScope {
    context_host: Rc<RefCell<JsContextHost>>,
    previous: Option<CurrentInputEvent>,
    installed: CurrentInputEvent,
}

impl CurrentInputEventScope {
    pub(crate) fn enter(
        context_host: Rc<RefCell<JsContextHost>>,
        event: CurrentInputEvent,
    ) -> Self {
        let previous = context_host
            .borrow_mut()
            .replace_current_input_event(Some(event));
        Self {
            context_host,
            previous,
            installed: event,
        }
    }
}

impl Drop for CurrentInputEventScope {
    fn drop(&mut self) {
        let replaced = self
            .context_host
            .borrow_mut()
            .replace_current_input_event(self.previous);
        assert_eq!(
            replaced,
            Some(self.installed),
            "the current input event must remain scoped to its real input dispatch"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ALT: u8 = 1;
    const CONTROL: u8 = 2;
    const META: u8 = 4;
    const SHIFT: u8 = 8;

    #[cfg(target_os = "macos")]
    const PLATFORM_NEW_TAB_MODIFIER: u8 = META;
    #[cfg(not(target_os = "macos"))]
    const PLATFORM_NEW_TAB_MODIFIER: u8 = CONTROL;

    #[cfg(target_os = "macos")]
    const NON_PLATFORM_NEW_TAB_MODIFIER: u8 = CONTROL;
    #[cfg(not(target_os = "macos"))]
    const NON_PLATFORM_NEW_TAB_MODIFIER: u8 = META;

    #[test]
    fn modifier_policy_matches_chromium_surface_selection() {
        assert_eq!(
            navigation_policy_from_modifiers(0, InputModifiers::from_bits(0)),
            InputNavigationPolicy::Current
        );
        assert_eq!(
            navigation_policy_from_modifiers(
                0,
                InputModifiers::from_bits(NON_PLATFORM_NEW_TAB_MODIFIER),
            ),
            InputNavigationPolicy::Current
        );
        assert_eq!(
            navigation_policy_from_modifiers(1, InputModifiers::from_bits(0)),
            InputNavigationPolicy::NewBackgroundSurface
        );
        assert_eq!(
            navigation_policy_from_modifiers(
                0,
                InputModifiers::from_bits(PLATFORM_NEW_TAB_MODIFIER),
            ),
            InputNavigationPolicy::NewBackgroundSurface
        );
        assert_eq!(
            navigation_policy_from_modifiers(0, InputModifiers::from_bits(SHIFT)),
            InputNavigationPolicy::NewWindow
        );
        assert_eq!(
            navigation_policy_from_modifiers(1, InputModifiers::from_bits(SHIFT)),
            InputNavigationPolicy::NewForegroundSurface
        );
        assert_eq!(
            navigation_policy_from_modifiers(0, InputModifiers::from_bits(ALT)),
            InputNavigationPolicy::Download
        );
    }

    #[test]
    fn synthesized_events_cannot_request_tab_unders_or_downloads() {
        assert_eq!(
            navigation_policy_from_event(1, 0, None),
            InputNavigationPolicy::NewForegroundSurface
        );
        assert_eq!(
            navigation_policy_from_event(0, PLATFORM_NEW_TAB_MODIFIER, None),
            InputNavigationPolicy::NewForegroundSurface
        );
        assert_eq!(
            navigation_policy_from_event(0, ALT, None),
            InputNavigationPolicy::Current
        );
    }

    #[test]
    fn only_navigation_capable_real_input_supplies_an_input_policy() {
        assert_eq!(
            CurrentInputEvent::mouse("mousedown", 0, PLATFORM_NEW_TAB_MODIFIER).navigation_policy(),
            InputNavigationPolicy::Current
        );
        assert_eq!(
            CurrentInputEvent::mouse("mouseup", 0, PLATFORM_NEW_TAB_MODIFIER).navigation_policy(),
            InputNavigationPolicy::NewBackgroundSurface
        );
        assert_eq!(
            CurrentInputEvent::keyboard("Enter", PLATFORM_NEW_TAB_MODIFIER).navigation_policy(),
            InputNavigationPolicy::NewBackgroundSurface
        );
        assert_eq!(
            CurrentInputEvent::keyboard(" ", PLATFORM_NEW_TAB_MODIFIER).navigation_policy(),
            InputNavigationPolicy::Current
        );
    }
}
