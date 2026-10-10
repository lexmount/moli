//! Native certificate operations supplied by the embedder for this realm.

pub type ValidateCertificates =
    for<'s, 'p> fn(&mut v8::PinScope<'s, 'p>, &[v8::Local<'s, v8::Object>]) -> Option<()>;
pub type InitializeConnection = for<'s, 'p> fn(
    &mut v8::PinScope<'s, 'p>,
    v8::Local<'s, v8::Object>,
    &[v8::Local<'s, v8::Object>],
) -> Option<()>;
pub type PreserveConnection =
    for<'s, 'p> fn(&mut v8::PinScope<'s, 'p>, v8::Local<'s, v8::Object>, v8::Local<'s, v8::Object>);
pub type ConnectionFingerprints =
    for<'s, 'p> fn(&mut v8::PinScope<'s, 'p>, v8::Local<'s, v8::Object>, &mut String);

#[derive(Clone, Copy)]
pub struct CertificateRuntime {
    pub validate_configuration: ValidateCertificates,
    pub initialize_connection: InitializeConnection,
    pub preserve_connection: PreserveConnection,
    pub connection_fingerprints: ConnectionFingerprints,
}

pub fn install_runtime(scope: &mut v8::PinScope<'_, '_>, runtime: CertificateRuntime) {
    scope.get_current_context().set_slot(runtime.into());
}

fn runtime(scope: &mut v8::PinScope<'_, '_>) -> CertificateRuntime {
    *scope
        .get_current_context()
        .get_slot::<CertificateRuntime>()
        .expect("install the certificate runtime before invoking RTC bindings")
}

pub(super) fn validate_configuration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    certificates: &[v8::Local<'s, v8::Object>],
) -> Option<()> {
    (runtime(scope).validate_configuration)(scope, certificates)
}

pub(super) fn initialize_connection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    config: v8::Local<'s, v8::Object>,
    certificates: &[v8::Local<'s, v8::Object>],
) -> Option<()> {
    (runtime(scope).initialize_connection)(scope, config, certificates)
}

pub(super) fn preserve_connection<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    old: v8::Local<'s, v8::Object>,
    next: v8::Local<'s, v8::Object>,
) {
    (runtime(scope).preserve_connection)(scope, old, next);
}

pub(super) fn connection_fingerprints<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    config: v8::Local<'s, v8::Object>,
    sdp: &mut String,
) {
    (runtime(scope).connection_fingerprints)(scope, config, sdp);
}
