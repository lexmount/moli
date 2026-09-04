use super::super::emulation::{
    EmulatedDeviceMetrics, EmulatedGeolocationOverrideState, EmulatedMediaOverrides,
    EmulatedNetworkConditions,
};

/// Source-free values applied to one Browser page. Session contributions use
/// the same value representation, but do not own the installed policy.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct EmulationPolicy {
    pub(crate) default_background_color: Option<[u8; 4]>,
    pub(crate) network_conditions: Option<EmulatedNetworkConditions>,
    pub(crate) geolocation_override: Option<EmulatedGeolocationOverrideState>,
    pub(crate) emulated_media: EmulatedMediaOverrides,
    pub(crate) emulated_device_metrics: Option<EmulatedDeviceMetrics>,
    pub(crate) max_touch_points: u32,
    pub(crate) emit_touch_events_for_mouse: bool,
    pub(crate) focus_emulation_enabled: bool,
    pub(crate) script_execution_disabled: bool,
}

#[derive(Debug, Clone)]
pub(crate) enum EmulationPolicyChange {
    DefaultBackgroundColor(Option<[u8; 4]>),
    NetworkConditions(Option<EmulatedNetworkConditions>),
    Geolocation(Option<EmulatedGeolocationOverrideState>),
    Media(EmulatedMediaOverrides),
    DeviceMetrics(Option<EmulatedDeviceMetrics>),
    MaxTouchPoints(u32),
    EmitTouchEventsForMouse(bool),
    FocusEnabled(bool),
    ScriptExecutionDisabled(bool),
}

impl EmulationPolicy {
    pub(in crate::conn) fn apply(&mut self, change: EmulationPolicyChange) {
        match change {
            EmulationPolicyChange::DefaultBackgroundColor(value) => {
                self.default_background_color = value
            }
            EmulationPolicyChange::NetworkConditions(value) => self.network_conditions = value,
            EmulationPolicyChange::Geolocation(value) => self.geolocation_override = value,
            EmulationPolicyChange::Media(value) => self.emulated_media = value,
            EmulationPolicyChange::DeviceMetrics(value) => self.emulated_device_metrics = value,
            EmulationPolicyChange::MaxTouchPoints(value) => self.max_touch_points = value,
            EmulationPolicyChange::EmitTouchEventsForMouse(value) => {
                self.emit_touch_events_for_mouse = value
            }
            EmulationPolicyChange::FocusEnabled(value) => self.focus_emulation_enabled = value,
            EmulationPolicyChange::ScriptExecutionDisabled(value) => {
                self.script_execution_disabled = value
            }
        }
    }

    pub(in crate::conn) fn apply_changes(
        &mut self,
        changes: Vec<EmulationPolicyChange>,
    ) -> EmulationPolicyDelta {
        let previous = self.clone();
        for change in changes {
            self.apply(change);
        }
        previous.delta(self)
    }

    fn delta(&self, next: &Self) -> EmulationPolicyDelta {
        EmulationPolicyDelta {
            navigator_queries: false,
            network_conditions: self.network_conditions != next.network_conditions,
            geolocation_override: self.geolocation_override != next.geolocation_override,
            emulated_media: self.emulated_media != next.emulated_media,
            emulated_device_metrics: self.emulated_device_metrics != next.emulated_device_metrics,
            max_touch_points: self.max_touch_points != next.max_touch_points,
            focus_emulation_enabled: self.focus_emulation_enabled != next.focus_emulation_enabled,
            script_execution_disabled: self.script_execution_disabled
                != next.script_execution_disabled,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct EmulationPolicyDelta {
    pub(crate) navigator_queries: bool,
    pub(crate) network_conditions: bool,
    pub(crate) geolocation_override: bool,
    pub(crate) emulated_media: bool,
    pub(crate) emulated_device_metrics: bool,
    pub(crate) max_touch_points: bool,
    pub(crate) focus_emulation_enabled: bool,
    pub(crate) script_execution_disabled: bool,
}

impl EmulationPolicyDelta {
    pub(crate) fn surface_changed(self) -> bool {
        self.network_conditions
            || self.geolocation_override
            || self.emulated_device_metrics
            || self.max_touch_points
            || self.navigator_queries
            || self.focus_emulation_enabled
    }
}
