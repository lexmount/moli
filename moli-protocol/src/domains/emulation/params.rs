pub(super) use chromiumoxide_cdp::cdp::browser_protocol::emulation::{
    ClearGeolocationOverrideParams, ClearIdleOverrideParams, SetAutomationOverrideParams,
    SetCpuThrottlingRateParams, SetDataSaverOverrideParams,
    SetDefaultBackgroundColorOverrideParams, SetEmitTouchEventsForMouseParams,
    SetEmulatedMediaParams, SetFocusEmulationEnabledParams, SetGeolocationOverrideParams,
    SetHardwareConcurrencyOverrideParams, SetIdleOverrideParams, SetLocaleOverrideParams,
    SetScriptExecutionDisabledParams, SetTimezoneOverrideParams, SetTouchEmulationEnabledParams,
};

use chromiumoxide_cdp::cdp::browser_protocol::{
    emulation::{DevicePosture, DisplayFeature, ScreenOrientation},
    page::Viewport,
};

// The generated bindings omit newer optional fields on this command. Keep
// them at the wire boundary so we can reject unsupported modes explicitly.
// Unknown extension fields remain ignorable, as in the other CDP commands.
#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct SetDeviceMetricsOverrideParams {
    pub width: i64,
    pub height: i64,
    pub device_scale_factor: f64,
    pub mobile: bool,
    pub scale: Option<f64>,
    pub screen_width: Option<i64>,
    pub screen_height: Option<i64>,
    pub position_x: Option<i64>,
    pub position_y: Option<i64>,
    pub dont_set_visible_size: Option<bool>,
    pub screen_orientation: Option<ScreenOrientation>,
    pub viewport: Option<Viewport>,
    pub display_feature: Option<DisplayFeature>,
    pub device_posture: Option<DevicePosture>,
    pub scrollbar_type: Option<ScrollbarType>,
    pub screen_orientation_lock_emulation: Option<bool>,
    pub viewport_meta: Option<ViewportMeta>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum ScrollbarType {
    Default,
    Overlay,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum ViewportMeta {
    Default,
    Enable,
}
