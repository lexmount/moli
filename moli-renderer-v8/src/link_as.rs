#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LinkAsDestination {
    None,
    Audio,
    AudioWorklet,
    Document,
    Embed,
    Fetch,
    Font,
    Frame,
    IFrame,
    Image,
    Json,
    Manifest,
    Object,
    PaintWorklet,
    Report,
    Script,
    ServiceWorker,
    SharedWorker,
    Style,
    Text,
    Track,
    Video,
    WebIdentity,
    Worker,
    Xslt,
}

impl LinkAsDestination {
    pub(crate) fn is_preload_destination(self) -> bool {
        // HTML's preload destinations are a subset of the enumerated `as`
        // keywords. Missing and unknown values have no preload default.
        matches!(
            self,
            Self::Fetch | Self::Font | Self::Image | Self::Script | Self::Style | Self::Track
        )
    }

    pub(crate) fn reflected_value(self) -> &'static str {
        match self {
            Self::None => "",
            Self::Audio => "audio",
            Self::AudioWorklet => "audioworklet",
            Self::Document => "document",
            Self::Embed => "embed",
            Self::Fetch => "fetch",
            Self::Font => "font",
            Self::Frame => "frame",
            Self::IFrame => "iframe",
            Self::Image => "image",
            Self::Json => "json",
            Self::Manifest => "manifest",
            Self::Object => "object",
            Self::PaintWorklet => "paintworklet",
            Self::Report => "report",
            Self::Script => "script",
            Self::ServiceWorker => "serviceworker",
            Self::SharedWorker => "sharedworker",
            Self::Style => "style",
            Self::Text => "text",
            Self::Track => "track",
            Self::Video => "video",
            Self::WebIdentity => "webidentity",
            Self::Worker => "worker",
            Self::Xslt => "xslt",
        }
    }
}

pub(crate) fn link_as_destination(value: Option<&str>) -> LinkAsDestination {
    let Some(value) = value else {
        return LinkAsDestination::None;
    };
    match value.to_ascii_lowercase().as_str() {
        "audio" => LinkAsDestination::Audio,
        "audioworklet" => LinkAsDestination::AudioWorklet,
        "document" => LinkAsDestination::Document,
        "embed" => LinkAsDestination::Embed,
        "fetch" => LinkAsDestination::Fetch,
        "font" => LinkAsDestination::Font,
        "frame" => LinkAsDestination::Frame,
        "iframe" => LinkAsDestination::IFrame,
        "image" => LinkAsDestination::Image,
        "json" => LinkAsDestination::Json,
        "manifest" => LinkAsDestination::Manifest,
        "object" => LinkAsDestination::Object,
        "paintworklet" => LinkAsDestination::PaintWorklet,
        "report" => LinkAsDestination::Report,
        "script" => LinkAsDestination::Script,
        "serviceworker" => LinkAsDestination::ServiceWorker,
        "sharedworker" => LinkAsDestination::SharedWorker,
        "style" => LinkAsDestination::Style,
        "text" => LinkAsDestination::Text,
        "track" => LinkAsDestination::Track,
        "video" => LinkAsDestination::Video,
        "webidentity" => LinkAsDestination::WebIdentity,
        "worker" => LinkAsDestination::Worker,
        "xslt" => LinkAsDestination::Xslt,
        _ => LinkAsDestination::None,
    }
}
