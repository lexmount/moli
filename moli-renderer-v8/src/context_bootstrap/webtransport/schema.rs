//! Complete WebIDL conversion precedes URL, protocol and session validation.

use crate::{web_api_interfaces, webidl};

#[derive(webidl::WebIdlEnum)]
#[webidl(name = "WebTransportCongestionControl", rename_all = "kebab-case")]
pub(super) enum CongestionControl {
    Default,
    Throughput,
    LowLatency,
}

#[derive(webidl::WebIdlEnum)]
#[webidl(name = "ReadableStreamType")]
pub(super) enum ReadableType {
    Bytes,
}

/// HeadersInit conversion retains the union arm. Pair length and HTTP syntax
/// belong to filling Headers, after the constructor's earlier validation.
#[derive(Default)]
pub(super) enum Headers {
    #[default]
    Empty,
    Sequence(Vec<webidl::Sequence<webidl::ByteString>>),
    Record(Vec<(webidl::ByteString, webidl::ByteString)>),
}

impl<'s> webidl::WebIdlConverter<'s> for Headers {
    type Options = ();

    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _: &(),
    ) -> Result<Self, webidl::WebIdlError> {
        if let Some(sequence) = webidl::convert_optional_sequence::<
            webidl::Sequence<webidl::ByteString>,
        >(scope, value, context, &webidl::StringOptions::default())?
        {
            return Ok(Self::Sequence(sequence.0));
        }
        webidl::convert::<webidl::Record<webidl::ByteString, webidl::ByteString>>(
            scope, value, context,
        )
        .map(|record| Self::Record(record.0))
    }
}

impl Headers {
    pub(super) fn validate(self, scope: &mut v8::PinScope<'_, '_>) -> bool {
        let pairs = match self {
            Self::Empty => Vec::new(),
            Self::Record(record) => record,
            Self::Sequence(sequence) => {
                let mut pairs = Vec::with_capacity(sequence.len());
                for pair in sequence {
                    let Ok([key, value]) = <[_; 2]>::try_from(pair.0) else {
                        crate::util::throw_type_error(scope, "Header pairs must have length 2.");
                        return false;
                    };
                    pairs.push((key, value));
                }
                pairs
            }
        };
        // Fill the complete Headers object before applying request-specific
        // restrictions; later invalid entries take precedence over these.
        let mut available_protocols = false;
        for (name, value) in pairs {
            let Some((name, _)) = crate::network_host::normalized_header_entry_or_throw(
                scope,
                name.into(),
                value.into(),
            ) else {
                return false;
            };
            available_protocols |= name == "wt-available-protocols";
        }
        if available_protocols {
            crate::util::throw_type_error(
                scope,
                "WT-Available-Protocols is controlled by WebTransport.",
            );
            return false;
        }
        true
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "WebTransportHash")]
pub(super) struct CertificateHash<'s> {
    #[webidl(required, converter = "raw", name = "algorithm")]
    _algorithm: webidl::DomString16,
    #[webidl(required, converter = "raw", name = "value")]
    _value: webidl::NonSharedBufferSource<'s>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "WebTransportOptions")]
pub(super) struct Options<'s> {
    #[webidl(default = false)]
    pub(super) allow_pooling: bool,
    #[webidl(nullable, converter = "enforce_range_unsigned_short")]
    pub(super) anticipated_concurrent_incoming_bidirectional_streams: Option<u16>,
    #[webidl(nullable, converter = "enforce_range_unsigned_short")]
    pub(super) anticipated_concurrent_incoming_unidirectional_streams: Option<u16>,
    #[webidl(name = "congestionControl", converter = "enum", default = CongestionControl::Default)]
    _congestion_control: CongestionControl,
    #[webidl(converter = "enum")]
    pub(super) datagrams_readable_type: Option<ReadableType>,
    #[webidl(converter = "raw", default = Headers::default())]
    pub(super) headers: Headers,
    #[webidl(sequence, converter = "raw", default = Vec::new())]
    pub(super) protocols: Vec<webidl::DomString16>,
    #[webidl(name = "requireUnreliable", default = false)]
    _require_unreliable: bool,
    #[webidl(sequence, converter = "dictionary", default = Vec::new())]
    pub(super) server_certificate_hashes: Vec<CertificateHash<'s>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WebTransport")]
pub(super) struct ConstructorArgs<'s> {
    #[webidl(required, converter = "usv_string")]
    pub(super) url: String,
    #[webidl(dictionary)]
    pub(super) options: Options<'s>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "WebTransportCloseInfo")]
struct CloseInfo {
    #[webidl(name = "closeCode", converter = "unsigned_long", default = 0)]
    _close_code: u32,
    #[webidl(name = "reason", converter = "usv_string", default = "")]
    _reason: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WebTransport.close")]
pub(super) struct CloseArgs {
    #[webidl(name = "closeInfo", dictionary)]
    _close_info: CloseInfo,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "WebTransportSendOptions")]
pub(super) struct SendOptions<'s> {
    #[webidl(nullable, interface = web_api_interfaces::WebTransportSendGroup)]
    pub(super) send_group: Option<v8::Local<'s, v8::Object>>,
    #[webidl(converter = "long_long", default = 0)]
    pub(super) send_order: i64,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "WebTransportSendStreamOptions")]
pub(super) struct SendStreamOptions<'s> {
    #[webidl(nullable, interface = web_api_interfaces::WebTransportSendGroup)]
    pub(super) send_group: Option<v8::Local<'s, v8::Object>>,
    #[webidl(name = "sendOrder", converter = "long_long", default = 0)]
    _send_order: i64,
    #[webidl(name = "waitUntilAvailable", default = false)]
    _wait_until_available: bool,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WebTransport.createStream")]
pub(super) struct CreateStreamArgs<'s> {
    #[webidl(dictionary)]
    pub(super) options: SendStreamOptions<'s>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WebTransportDatagramDuplexStream.createWritable")]
pub(super) struct CreateWritableArgs<'s> {
    #[webidl(dictionary)]
    pub(super) options: SendOptions<'s>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WebTransport.exportKeyingMaterial")]
pub(super) struct ExportArgs<'s> {
    #[webidl(required, converter = "raw")]
    pub(super) label: webidl::NonSharedBufferSource<'s>,
    #[webidl(required, converter = "raw")]
    pub(super) context: webidl::NonSharedBufferSource<'s>,
    #[webidl(required, converter = "unsigned_long")]
    pub(super) output_length: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WebTransport.anticipatedConcurrentIncomingStreams")]
pub(super) struct LimitArgs {
    #[webidl(nullable, converter = "enforce_range_unsigned_short")]
    pub(super) value: Option<u16>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WebTransportDatagramDuplexStream.maxAge")]
pub(super) struct AgeArgs {
    #[webidl(nullable, converter = "unrestricted_double")]
    pub(super) value: Option<f64>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WebTransportDatagramDuplexStream.maxBufferedDatagrams")]
pub(super) struct BufferedArgs {
    #[webidl(converter = "unsigned_long", default = 0)]
    pub(super) value: u32,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WebTransportDatagramsWritable.sendGroup")]
pub(super) struct GroupArgs<'s> {
    #[webidl(nullable, interface = web_api_interfaces::WebTransportSendGroup)]
    pub(super) value: Option<v8::Local<'s, v8::Object>>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "WebTransportDatagramsWritable.sendOrder")]
pub(super) struct OrderArgs {
    #[webidl(converter = "long_long", default = 0)]
    pub(super) value: i64,
}
