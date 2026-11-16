pub(super) use moli_webapi::geolocation::install;
use moli_webapi::geolocation::{Coordinates, build_position};

pub(super) fn build<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    position: &moli_page_types::GeolocationPositionOverride,
) -> v8::Local<'s, v8::Object> {
    let coordinates = Coordinates {
        latitude: position.latitude,
        longitude: position.longitude,
        altitude: position.altitude,
        accuracy: position.accuracy,
        altitude_accuracy: position.altitude_accuracy,
        heading: position.heading,
        speed: position.speed,
    };
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or_default();
    build_position(scope, &coordinates, timestamp)
}
