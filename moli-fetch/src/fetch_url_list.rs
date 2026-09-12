use moli_cookie_jar::same_site_urls;
use moli_url::{WebOrigin, same_origin};
use url::Url;

use crate::{RedirectInfo, RedirectSource, RequestMode};

/// A borrowed view of the URLs visited by a fetch. Requests and responses use
/// the same rules for redirect taint, including synthetic redirects.
#[derive(Clone, Copy, Debug)]
pub struct FetchUrlList<'a> {
    current_url: &'a Url,
    redirects: &'a [RedirectInfo],
}

impl<'a> FetchUrlList<'a> {
    pub fn new(current_url: &'a Url, redirects: &'a [RedirectInfo]) -> Self {
        Self {
            current_url,
            redirects,
        }
    }

    fn urls(self) -> impl Iterator<Item = &'a Url> {
        self.redirects
            .iter()
            .flat_map(|redirect| [&redirect.from_url, &redirect.to_url])
            .chain(std::iter::once(self.current_url))
    }

    /// Returning to the initiating origin cannot restore basic response tainting.
    pub fn has_cross_origin_url(self, origin: &WebOrigin) -> bool {
        self.urls().any(|url| !origin.same_origin(&url.into()))
    }

    /// Enforces same-origin mode before dispatch, including after redirects.
    /// Main fetch permits data URLs regardless of mode. CORS response checks
    /// are separate: permission from a server cannot relax same-origin mode.
    pub fn validate_request_mode(
        self,
        mode: RequestMode,
        origin: &WebOrigin,
    ) -> Result<(), String> {
        if mode == RequestMode::SameOrigin
            && self.current_url.scheme() != "data"
            && self.has_cross_origin_url(origin)
        {
            return Err(format!(
                "same-origin request mode blocked a cross-origin URL in the fetch of {}",
                self.current_url
            ));
        }
        Ok(())
    }

    pub fn has_cross_site_url(self, origin: &Url) -> bool {
        self.urls().any(|url| !same_site_urls(origin, url, true))
    }

    /// A first hop out of the initiating origin retains that origin. Crossing
    /// origins from an already cross-origin URL serializes the origin as null.
    pub fn serialized_origin(self, origin: &WebOrigin) -> String {
        if self.redirects.iter().any(|redirect| {
            !same_origin(&redirect.from_url, &redirect.to_url)
                && !origin.same_origin(&(&redirect.from_url).into())
        }) {
            "null".to_owned()
        } else {
            origin.ascii_serialization().to_owned()
        }
    }

    pub fn redirect_count(self) -> usize {
        self.redirects
            .iter()
            .filter(|redirect| redirect.source != RedirectSource::Internal)
            .count()
    }
}
