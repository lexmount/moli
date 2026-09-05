use super::super::emulation::{
    EmulatedDeviceMetrics, EmulatedGeolocationOverrideState, EmulatedMediaOverrides,
    EmulatedNetworkConditions,
};

/// Source-free values applied to one Browser page. Session contributions use
/// the same value representation, but do not own the installed policy.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct EmulationPolicy {
    pub(crate) network_conditions: Option<EmulatedNetworkConditions>,
    pub(crate) geolocation_override: Option<EmulatedGeolocationOverrideState>,
    pub(crate) emulated_media: EmulatedMediaOverrides,
    pub(crate) emulated_device_metrics: Option<EmulatedDeviceMetrics>,
    pub(crate) cpu_throttling_rate: f64,
    pub(crate) touch_emulation_enabled: bool,
    pub(crate) emit_touch_events_for_mouse: bool,
    pub(crate) focus_emulation_enabled: bool,
    pub(crate) script_execution_disabled: bool,
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
pub(crate) enum EmulationPolicyChange {
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
    pub(in crate::conn) fn apply(&mut self, change: EmulationPolicyChange) {
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
            network_conditions: self.network_conditions != next.network_conditions,
            geolocation_override: self.geolocation_override != next.geolocation_override,
            emulated_media: self.emulated_media != next.emulated_media,
            emulated_device_metrics: self.emulated_device_metrics != next.emulated_device_metrics,
            cpu_throttling_rate: self.cpu_throttling_rate != next.cpu_throttling_rate,
            touch_emulation_enabled: self.touch_emulation_enabled != next.touch_emulation_enabled,
            focus_emulation_enabled: self.focus_emulation_enabled != next.focus_emulation_enabled,
            script_execution_disabled: self.script_execution_disabled
                != next.script_execution_disabled,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct EmulationPolicyDelta {
    pub(crate) network_conditions: bool,
    pub(crate) geolocation_override: bool,
    pub(crate) emulated_media: bool,
    pub(crate) emulated_device_metrics: bool,
    pub(crate) cpu_throttling_rate: bool,
    pub(crate) touch_emulation_enabled: bool,
    pub(crate) focus_emulation_enabled: bool,
    pub(crate) script_execution_disabled: bool,
}

impl EmulationPolicyDelta {
    pub(crate) fn surface_changed(self) -> bool {
        self.network_conditions
            || self.geolocation_override
            || self.emulated_device_metrics
            || self.touch_emulation_enabled
            || self.focus_emulation_enabled
    }
}
