pub(super) use crate::conn::LIVE_DEVICE_METRICS_CLEAR_SCRIPT;
use crate::conn::{EmulatedDeviceMetrics, viewport_surface_install_script};

pub(super) fn live_device_metrics_override_script(
    metrics: &EmulatedDeviceMetrics,
    remember_original_descriptors: bool,
) -> String {
    viewport_surface_install_script(&metrics.viewport_surface(), remember_original_descriptors)
}

#[cfg(test)]
mod tests {
    use super::LIVE_DEVICE_METRICS_CLEAR_SCRIPT;

    #[test]
    fn live_device_metrics_clear_script_uses_plain_helper_store() {
        assert!(!LIVE_DEVICE_METRICS_CLEAR_SCRIPT.contains("Object.create(null)"));
        assert!(LIVE_DEVICE_METRICS_CLEAR_SCRIPT.contains("globalThis.parent === globalThis"));
        assert!(LIVE_DEVICE_METRICS_CLEAR_SCRIPT.contains("if (isTopLevelWindow)"));
    }
}
