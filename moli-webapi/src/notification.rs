use moli_webidl as webidl;

#[derive(Clone, Debug)]
pub struct NotificationMetadata {
    pub dir: String,
    pub lang: String,
    pub body: String,
    pub icon: String,
    pub image: String,
    pub badge: String,
    pub vibrate: Vec<u32>,
    pub timestamp: Option<u64>,
    pub renotify: bool,
    pub silent: Option<bool>,
    pub require_interaction: bool,
}

impl Default for NotificationMetadata {
    fn default() -> Self {
        Self {
            dir: "auto".to_owned(),
            lang: String::new(),
            body: String::new(),
            icon: String::new(),
            image: String::new(),
            badge: String::new(),
            vibrate: Vec::new(),
            timestamp: None,
            renotify: false,
            silent: None,
            require_interaction: false,
        }
    }
}

#[derive(webidl::WebIdlEnum)]
#[webidl(name = "NotificationDirection")]
enum NotificationDirection {
    #[webidl(token = "auto")]
    Auto,
    #[webidl(token = "ltr")]
    Ltr,
    #[webidl(token = "rtl")]
    Rtl,
}

struct VibrationPattern(Vec<u32>);

impl<'s> webidl::WebIdlConverter<'s> for VibrationPattern {
    type Options = ();
    fn convert(
        scope: &mut v8::PinScope<'s, '_>,
        value: v8::Local<'s, v8::Value>,
        context: webidl::Context,
        _: &(),
    ) -> Result<Self, webidl::WebIdlError> {
        if let Some(values) = webidl::convert_optional_sequence::<webidl::UnsignedLong>(
            scope,
            value,
            context,
            &Default::default(),
        )? {
            return Ok(Self(values.0.into_iter().map(|value| value.0).collect()));
        }
        webidl::convert::<webidl::UnsignedLong>(scope, value, context)
            .map(|value| Self(vec![value.0]))
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "NotificationAction")]
pub struct NotificationActionInit {
    #[webidl(required)]
    pub action: String,
    #[webidl(default = "", converter = "usv_string")]
    pub icon: String,
    #[webidl(converter = "usv_string")]
    pub navigate: Option<String>,
    #[webidl(required)]
    pub title: String,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "Notification")]
pub struct NotificationArgs<'s> {
    #[webidl(required)]
    pub title: String,
    #[webidl(dictionary)]
    pub options: NotificationOptionsInit<'s>,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "NotificationOptions")]
pub struct NotificationOptionsInit<'s> {
    #[webidl(sequence, converter = "raw", default = Vec::new())]
    pub actions: Vec<webidl::Dictionary<NotificationActionInit>>,
    #[webidl(default = "", converter = "usv_string")]
    badge: String,
    #[webidl(default = "")]
    body: String,
    #[webidl(converter = "raw")]
    pub data: Option<v8::Local<'s, v8::Value>>,
    #[webidl(default = NotificationDirection::Auto, converter = "enum")]
    dir: NotificationDirection,
    #[webidl(default = "", converter = "usv_string")]
    pub icon: String,
    #[webidl(default = "", converter = "usv_string")]
    image: String,
    #[webidl(default = "")]
    lang: String,
    #[webidl(default = false)]
    renotify: bool,
    #[webidl(default = false)]
    require_interaction: bool,
    #[webidl(nullable)]
    silent: Option<bool>,
    #[webidl(default = "")]
    pub tag: String,
    timestamp: Option<u64>,
    #[webidl(default = VibrationPattern(Vec::new()), converter = "raw")]
    vibrate: VibrationPattern,
}

impl NotificationOptionsInit<'_> {
    pub fn into_metadata(self) -> NotificationMetadata {
        NotificationMetadata {
            dir: match self.dir {
                NotificationDirection::Auto => "auto",
                NotificationDirection::Ltr => "ltr",
                NotificationDirection::Rtl => "rtl",
            }
            .to_owned(),
            lang: self.lang,
            body: self.body,
            icon: self.icon,
            image: self.image,
            badge: self.badge,
            vibrate: self.vibrate.0,
            timestamp: self.timestamp,
            renotify: self.renotify,
            silent: self.silent,
            require_interaction: self.require_interaction,
        }
    }
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "GetNotificationOptions")]
struct GetNotificationOptions {
    #[webidl(default = "")]
    tag: String,
}

pub fn notification_get_options_tag<'s>(
    scope: &mut v8::PinScope<'s, '_>,
    value: v8::Local<'s, v8::Value>,
) -> Option<Option<String>> {
    match webidl::convert::<webidl::Dictionary<GetNotificationOptions>>(
        scope,
        value,
        webidl::Context::argument("ServiceWorkerRegistration.getNotifications", 1),
    ) {
        Ok(parsed) => Some((!parsed.0.tag.is_empty()).then_some(parsed.0.tag)),
        Err(error) => {
            webidl::throw_error(scope, &error);
            None
        }
    }
}
