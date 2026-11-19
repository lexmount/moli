use moli_webidl as webidl;

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "PermissionDescriptor")]
pub struct PermissionDescriptorMembers {
    #[webidl(required)]
    pub name: String,
}

#[derive(webidl::WebIdlDictionary)]
#[webidl(prefix = "MidiPermissionDescriptor")]
pub struct MidiPermissionDescriptorMembers {
    #[webidl(required)]
    pub name: String,

    #[webidl(default = false)]
    pub sysex: bool,
}

#[derive(Default, webidl::WebIdlDictionary)]
#[webidl(prefix = "ClipboardUnsanitizedFormats")]
pub struct ClipboardUnsanitizedFormats {
    #[webidl(sequence)]
    pub unsanitized: Option<Vec<String>>,
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "MediaSessionAction")]
pub enum MediaSessionAction {
    Play,
    Pause,
    Seekbackward,
    Seekforward,
    Previoustrack,
    Nexttrack,
    Skipad,
    Stop,
    Seekto,
    Togglemicrophone,
    Togglecamera,
    Togglescreenshare,
    Hangup,
    Previousslide,
    Nextslide,
    Enterpictureinpicture,
    Voiceactivity,
}

#[derive(Clone, Copy, webidl::WebIdlEnum)]
#[webidl(name = "MediaSessionPlaybackState")]
pub enum PlaybackState {
    None,
    Paused,
    Playing,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaSession.setActionHandler")]
pub struct SetActionHandlerArgs {
    #[webidl(required, converter = "enum")]
    pub action: MediaSessionAction,
    #[webidl(required, nullable, converter = "callback_function")]
    pub handler: Option<webidl::WebIdlCallbackFunction>,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "MediaSession.playbackState")]
pub struct PlaybackStateArgs {
    #[webidl(required)]
    pub value: String,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, webidl::WebIdlEnum)]
#[webidl(name = "OrientationLockType", rename_all = "kebab-case")]
pub enum OrientationLockType {
    Any,
    Natural,
    Landscape,
    Portrait,
    PortraitPrimary,
    PortraitSecondary,
    LandscapePrimary,
    LandscapeSecondary,
}

#[derive(webidl::WebIdlArgs)]
#[webidl(prefix = "ScreenOrientation.lock")]
pub struct ScreenOrientationLockArgs {
    #[webidl(required, converter = "enum")]
    pub orientation: OrientationLockType,
}
