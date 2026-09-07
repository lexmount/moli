use crate::browser::{
    EmulatedDeviceMetrics, EmulatedGeolocationOverrideState, EmulatedMediaOverrides,
    EmulatedNetworkConditions,
};

/// Source-free values applied to one Browser page. Session contributions use
/// the same value representation, but do not own the installed policy.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct EmulationPolicy {
    pub default_background_color: Option<[u8; 4]>,
    pub network_conditions: Option<EmulatedNetworkConditions>,
    pub geolocation_override: Option<EmulatedGeolocationOverrideState>,
    pub emulated_media: EmulatedMediaOverrides,
    pub emulated_device_metrics: Option<EmulatedDeviceMetrics>,
    pub max_touch_points: u32,
    pub emit_touch_events_for_mouse: bool,
    pub focus_emulation_enabled: bool,
    pub script_execution_disabled: bool,
}

#[derive(Debug, Clone)]
pub enum EmulationPolicyChange {
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
    pub fn apply(&mut self, change: EmulationPolicyChange) {
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

    pub fn apply_changes(&mut self, changes: Vec<EmulationPolicyChange>) {
        for change in changes {
            self.apply(change);
        }
    }
}
