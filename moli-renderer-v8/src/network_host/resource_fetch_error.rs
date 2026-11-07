#[derive(Debug, thiserror::Error)]
pub(crate) enum ResourceFetchError {
    #[error("resource load sequence is no longer pending")]
    SequenceNotPending,
    #[error("resource request owner is stale")]
    StaleOwner,
    #[error("Document resource loader is unavailable")]
    NoResourceLoader,
    #[error("Document request environment is unavailable")]
    NoRequestEnvironment,
    #[error("media element is unavailable")]
    NoMediaElement,
    #[error("media lifecycle target is not an audio or video element")]
    InvalidMediaElement,
    #[error("media resource selection found no supported URL")]
    NoMediaSource,
    #[error("resource owner changed before request binding")]
    OwnerChangedBeforeBinding,
    #[error("FontFace URL loader is unavailable in this realm")]
    NoFontFaceRealm,
    #[error("resource request blocked by Content Security Policy")]
    ContentSecurityPolicyBlocked,
    #[error("resource request returned HTTP {status}")]
    HttpStatus { status: u16 },
}
