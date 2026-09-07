use crate::browser::{
    EmulatedDeviceMetrics, EmulatedGeolocationOverrideState, EmulatedMediaOverrides,
    EmulatedNetworkConditions,
};

/// Source-free values applied to one Browser page. Session contributions use
/// the same value representation, but do not own the installed policy.
#[derive(Debug, Clone, PartialEq)]
pub struct EmulationPolicy {
    pub network_conditions: Option<EmulatedNetworkConditions>,
    pub geolocation_override: Option<EmulatedGeolocationOverrideState>,
    pub emulated_media: EmulatedMediaOverrides,
    pub emulated_device_metrics: Option<EmulatedDeviceMetrics>,
    pub cpu_throttling_rate: f64,
    pub touch_emulation_enabled: bool,
    pub emit_touch_events_for_mouse: bool,
    pub focus_emulation_enabled: bool,
    pub script_execution_disabled: bool,
}

impl Default for EmulationPolicy {
    fn default() -> Self {
        Self {
            network_conditions: None,
            geolocation_override: None,
            emulated_media: EmulatedMediaOverrides::default(),
            emulated_device_metrics: None,
            cpu_throttling_rate: 1.0,
            touch_emulation_enabled: false,
            emit_touch_events_for_mouse: false,
            focus_emulation_enabled: false,
            script_execution_disabled: false,
        }
    }
}

#[derive(Debug, Clone)]
pub enum EmulationPolicyChange {
    NetworkConditions(Option<EmulatedNetworkConditions>),
    Geolocation(Option<EmulatedGeolocationOverrideState>),
    Media(EmulatedMediaOverrides),
    DeviceMetrics(Option<EmulatedDeviceMetrics>),
    CpuThrottlingRate(f64),
    TouchEnabled(bool),
    EmitTouchEventsForMouse(bool),
    FocusEnabled(bool),
    ScriptExecutionDisabled(bool),
}

impl EmulationPolicy {
    pub fn apply(&mut self, change: EmulationPolicyChange) {
        match change {
            EmulationPolicyChange::NetworkConditions(value) => self.network_conditions = value,
            EmulationPolicyChange::Geolocation(value) => self.geolocation_override = value,
            EmulationPolicyChange::Media(value) => self.emulated_media = value,
            EmulationPolicyChange::DeviceMetrics(value) => self.emulated_device_metrics = value,
            EmulationPolicyChange::CpuThrottlingRate(value) => self.cpu_throttling_rate = value,
            EmulationPolicyChange::TouchEnabled(value) => self.touch_emulation_enabled = value,
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
