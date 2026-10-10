//! Configuration dictionaries and snapshots, independent of an ICE transport.
//! Argument conversion completes before inspecting or changing connection state.

use super::{RTC_PEER_CONNECTION_CONFIGURATION_SLOT, RTC_PEER_CONNECTION_SIGNALING_STATE_SLOT};
use crate::{
    util::{get_private_value, v8str},
    web_api_interfaces, webidl,
};
use moli_webapi_declare::WebApiValue;
use webidl::{SnapshotKind, SnapshotPrototype, WebIdlDictionarySerialize, WebIdlSnapshotValue};

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "RTCBundlePolicy")]
enum BundlePolicy {
    #[webidl(token = "balanced")]
    Balanced,
    #[webidl(token = "max-compat")]
    MaxCompat,
    #[webidl(token = "max-bundle")]
    MaxBundle,
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "RTCIceTransportPolicy")]
enum IceTransportPolicy {
    #[webidl(token = "all")]
    All,
    #[webidl(token = "relay")]
    Relay,
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "RTCRtcpMuxPolicy")]
enum RtcpMuxPolicy {
    #[webidl(token = "require")]
    Require,
}

struct PoolSize(u8);

impl WebIdlSnapshotValue for PoolSize {
    const SNAPSHOT_KIND: SnapshotKind = SnapshotKind::Scalar;
}

impl<'s> webidl::WebIdlConverter<'s> for PoolSize {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _: &(),
    ) -> Result<Self, webidl::WebIdlError> {
        let value = webidl::convert::<webidl::EnforceRangeUnsignedLong>(scope, value, context)?.0;
        u8::try_from(value)
            .map(Self)
            .map_err(|_| webidl::WebIdlError::cannot_convert(context, "[EnforceRange] octet"))
    }
}

impl<'s> WebApiValue<'s> for PoolSize {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        self.0.to_v8_value(scope)
    }
}

struct Text(webidl::DomString16);

impl WebIdlSnapshotValue for Text {
    const SNAPSHOT_KIND: SnapshotKind = SnapshotKind::Scalar;
}

impl<'s> webidl::WebIdlConverter<'s> for Text {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _: &(),
    ) -> Result<Self, webidl::WebIdlError> {
        webidl::convert::<webidl::DomString16>(scope, value, context).map(Self)
    }
}

impl<'s> WebApiValue<'s> for Text {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        v8::String::new_from_two_byte(scope, &self.0.0, v8::NewStringType::Normal).map(Into::into)
    }
}

struct IceUrls(Vec<String>);

impl WebIdlSnapshotValue for IceUrls {
    // The union converter always binds its canonical value as a URL sequence.
    const SNAPSHOT_KIND: SnapshotKind = SnapshotKind::Sequence(&SnapshotKind::Scalar);
}

impl<'s> webidl::WebIdlConverter<'s> for IceUrls {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _: &(),
    ) -> Result<Self, webidl::WebIdlError> {
        // Union discrimination must not read @@iterator twice or iterate strings.
        if let Some(values) = webidl::convert_optional_sequence::<webidl::UsvString>(
            scope,
            value,
            context,
            &Default::default(),
        )? {
            return Ok(Self(values.0.into_iter().map(|value| value.0).collect()));
        }
        let value = webidl::convert::<webidl::UsvString>(scope, value, context)?.0;
        Ok(Self(vec![value]))
    }
}

impl<'s> WebApiValue<'s> for IceUrls {
    fn to_v8_value(&self, scope: &mut v8::PinScope<'s, '_>) -> Option<v8::Local<'s, v8::Value>> {
        self.0.to_v8_value(scope)
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RTCIceServer", snapshot)]
struct IceServer {
    #[webidl(converter = "raw")]
    credential: Option<Text>,
    #[webidl(required, converter = "raw")]
    urls: IceUrls,
    #[webidl(converter = "raw")]
    username: Option<Text>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "RTCConfiguration", snapshot)]
pub(super) struct Configuration<'s> {
    #[webidl(converter = "enum", default = BundlePolicy::Balanced)]
    bundle_policy: BundlePolicy,
    #[webidl(sequence, interface = web_api_interfaces::RTCCertificate, default = Vec::new())]
    certificates: Vec<v8::Local<'s, v8::Object>>,
    #[webidl(converter = "raw", default = PoolSize(0))]
    ice_candidate_pool_size: PoolSize,
    #[webidl(sequence, converter = "dictionary", default = Vec::new())]
    ice_servers: Vec<IceServer>,
    #[webidl(converter = "enum", default = IceTransportPolicy::All)]
    ice_transport_policy: IceTransportPolicy,
    #[webidl(converter = "enum", default = RtcpMuxPolicy::Require)]
    rtcp_mux_policy: RtcpMuxPolicy,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "RTCPeerConnection")]
struct ConfigurationArgs<'s> {
    #[webidl(dictionary)]
    configuration: Configuration<'s>,
}

pub(super) type ConfigurationSnapshot<'s> = webidl::DictionarySnapshot<'s, Configuration<'s>>;

pub(super) fn constructor_configuration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: &v8::FunctionCallbackArguments<'s>,
) -> Option<ConfigurationSnapshot<'s>> {
    let parsed = webidl::parse_args::<ConfigurationArgs>(scope, args)?;
    super::certificate::validate_configuration(scope, &parsed.configuration.certificates)?;
    validate_servers(scope, &parsed.configuration)?;
    let snapshot = parsed.configuration.to_snapshot(scope)?;
    let record = snapshot.to_v8_value(scope)?.try_into().ok()?;
    super::certificate::initialize_connection(scope, record, &parsed.configuration.certificates)?;
    Some(snapshot)
}

fn target<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    receiver: v8::Local<'s, v8::Object>,
) -> v8::Local<'s, v8::Object> {
    moli_webapi_declare::web_api_object_target(scope, receiver)
        .expect("validated RTCPeerConnection")
}

pub(super) fn configuration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    target: v8::Local<'s, v8::Object>,
) -> ConfigurationSnapshot<'s> {
    ConfigurationSnapshot::from_private_slot(scope, target, RTC_PEER_CONNECTION_CONFIGURATION_SLOT)
        .expect("native RTCConfiguration snapshot")
}

pub(super) fn get_configuration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    mut rv: v8::ReturnValue<'_, v8::Value>,
) {
    let target = target(scope, args.this());
    let stored = configuration(scope, target);
    let snapshot = stored
        .to_js_snapshot(scope, SnapshotPrototype::Ordinary)
        .expect("native RTCConfiguration snapshot");
    rv.set(snapshot.into());
}

pub(super) fn set_configuration<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    args: v8::FunctionCallbackArguments<'s>,
    _rv: v8::ReturnValue<'_, v8::Value>,
) {
    let Some(parsed) = webidl::parse_args::<ConfigurationArgs>(scope, &args) else {
        return;
    };
    let target = target(scope, args.this());
    let state = get_private_value(scope, target, RTC_PEER_CONNECTION_SIGNALING_STATE_SLOT)
        .expect("native signaling state");
    if state.strict_equals(v8str(scope, "closed").into()) {
        webidl::throw_dom_exception(scope, "InvalidStateError", "The connection is closed.");
        return;
    }
    let old = configuration(scope, target)
        .to_v8_value(scope)
        .unwrap()
        .try_into()
        .unwrap();
    let snapshot = parsed
        .configuration
        .to_snapshot(scope)
        .expect("converted RTCConfiguration");
    let next = snapshot.to_v8_value(scope).unwrap().try_into().unwrap();
    let local_description = super::signaling::local_description(scope, target);
    let unchanged = ["bundlePolicy", "rtcpMuxPolicy"]
        .into_iter()
        .all(|name| property(scope, old, name).strict_equals(property(scope, next, name)))
        && same_certificates(scope, old, next)
        && (local_description.is_null()
            || property(scope, old, "iceCandidatePoolSize").strict_equals(property(
                scope,
                next,
                "iceCandidatePoolSize",
            )));
    if !unchanged {
        webidl::throw_dom_exception(
            scope,
            "InvalidModificationError",
            "An immutable configuration member changed.",
        );
        return;
    }
    if validate_servers(scope, &parsed.configuration).is_none() {
        return;
    }
    super::certificate::preserve_connection(scope, old, next);
    snapshot
        .store_private(scope, target, RTC_PEER_CONNECTION_CONFIGURATION_SLOT)
        .expect("RTCConfiguration snapshot slot");
}

fn validate_servers(scope: &mut v8::PinScope<'_, '_>, config: &Configuration<'_>) -> Option<()> {
    for server in &config.ice_servers {
        if server.urls.0.is_empty() {
            webidl::throw_dom_exception(
                scope,
                "SyntaxError",
                "An ICE server needs at least one URL.",
            );
            return None;
        }
        for value in &server.urls.0 {
            let Ok(turn) = validate_url(value) else {
                webidl::throw_dom_exception(scope, "SyntaxError", "Invalid ICE server URL.");
                return None;
            };
            if turn
                && (server
                    .username
                    .as_ref()
                    .is_none_or(|value| String::from_utf16_lossy(&value.0.0).len() > 509)
                    || server
                        .credential
                        .as_ref()
                        .is_none_or(|value| value.0.0.is_empty()))
            {
                webidl::throw_dom_exception(
                    scope,
                    "InvalidAccessError",
                    "Invalid TURN username or credential.",
                );
                return None;
            }
        }
    }
    Some(())
}

// WebRTC-PC's ICE URL validation algorithm parses the opaque URI first,
// then validates its host/port using the URL parser. It is not a generic
// hierarchical URL: paths, userinfo, fragments and arbitrary queries fail.
fn validate_url(value: &str) -> Result<bool, ()> {
    let uri = url::Url::parse(value).map_err(|_| ())?;
    let turn = match uri.scheme() {
        "stun" | "stuns" => false,
        "turn" | "turns" => true,
        _ => return Err(()),
    };
    if !uri.cannot_be_a_base()
        || uri.path().contains(['/', '\\', '@'])
        || uri.fragment().is_some()
        || uri
            .query()
            .is_some_and(|query| !turn || !matches!(query, "transport=udp" | "transport=tcp"))
    {
        return Err(());
    }
    let authority = url::Url::parse(&format!("https://{}", uri.path())).map_err(|_| ())?;
    if authority.host().is_none()
        || authority.path() != "/"
        || !authority.username().is_empty()
        || authority.password().is_some()
        || authority.query().is_some()
        || authority.fragment().is_some()
    {
        return Err(());
    }
    Ok(turn)
}

fn property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> v8::Local<'s, v8::Value> {
    object
        .get(scope, v8str(scope, name).into())
        .expect("private dictionary data")
}

fn array_property<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    object: v8::Local<'s, v8::Object>,
    name: &'static str,
) -> v8::Local<'s, v8::Array> {
    v8::Local::try_from(property(scope, object, name)).expect("private sequence")
}

fn same_certificates<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    old: v8::Local<'s, v8::Object>,
    next: v8::Local<'s, v8::Object>,
) -> bool {
    let old = array_property(scope, old, "certificates");
    let next = array_property(scope, next, "certificates");
    old.length() == next.length()
        && (0..old.length()).all(|index| {
            old.get_index(scope, index)
                .expect("certificate")
                .strict_equals(next.get_index(scope, index).expect("certificate"))
        })
}
