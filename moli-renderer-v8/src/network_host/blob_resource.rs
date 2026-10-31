use std::sync::Arc;

use moli_file_api::{ObjectUrlResourceData, ObjectUrlResourceEntry};
use url::Url;

use super::{BlobUrlFetchEnvironment, LocalUrlError};

/// A Blob URL resolved at resource initiation, including an unsuccessful lookup.
/// This value can cross an owner handoff without retaining a V8 object or
/// starting file/database I/O. It belongs to the request, not the URL store.
#[derive(Clone, Debug)]
pub struct CapturedBlobResource {
    inner: Arc<CapturedBlobResourceData>,
}

#[derive(Debug)]
struct CapturedBlobResourceData {
    url: Url,
    entry: Option<ObjectUrlResourceEntry<crate::blob::ObjectUrlAccessKey>>,
}

impl CapturedBlobResource {
    pub(crate) fn capture(url: &Url) -> Option<Self> {
        (url.scheme() == "blob").then(|| Self {
            inner: Arc::new(CapturedBlobResourceData {
                url: url.clone(),
                entry: crate::blob::object_url_resource_entry(url.as_str()),
            }),
        })
    }

    pub(crate) fn capture_serialized(url: &str) -> Option<Self> {
        if !url
            .get(..5)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("blob:"))
        {
            return None;
        }
        Self::capture(&Url::parse(url).ok()?)
    }

    pub(crate) fn for_fetch_environment(
        self,
        environment: Option<&BlobUrlFetchEnvironment>,
    ) -> Self {
        let allowed = self.inner.entry.as_ref().is_some_and(|entry| {
            entry.access_key.as_ref().is_some_and(|key| {
                environment.is_some_and(|environment| environment.allows(key, &self.inner.url))
            })
        });
        if allowed || self.inner.entry.is_none() {
            return self;
        }
        Self {
            inner: Arc::new(CapturedBlobResourceData {
                url: self.inner.url.clone(),
                entry: None,
            }),
        }
    }

    pub(crate) fn matches(&self, url: &Url) -> bool {
        blob_url_key(&self.inner.url) == blob_url_key(url)
    }

    /// Consume this request's resolved backing, preserving method and range
    /// checks. A matching failed resolution remains an error after revocation;
    /// `None` means the caller is loading a different URL.
    pub async fn response(
        &self,
        url: &Url,
        method: &str,
        headers: &[(String, String)],
    ) -> Option<anyhow::Result<moli_fetch::Response>> {
        if !self.matches(url) {
            return None;
        }
        Some(
            self.read_response(url, method, headers)
                .await
                .map_err(Into::into),
        )
    }

    pub(crate) fn response_blocking(
        &self,
        url: &Url,
    ) -> Option<Result<moli_fetch::Response, LocalUrlError>> {
        if !self.matches(url) {
            return None;
        }
        Some((|| {
            let entry = self
                .inner
                .entry
                .as_ref()
                .ok_or_else(|| LocalUrlError::BlobUnavailable { url: url.clone() })?;
            let ObjectUrlResourceData::Blob { data, mime_type } = &entry.data else {
                return Err(LocalUrlError::MediaSource);
            };
            let bytes = data
                .read_blocking()
                .map_err(|_| LocalUrlError::BlobUnavailable { url: url.clone() })?;
            super::browser_response::blob_response(url, &bytes, mime_type, &[])
        })())
    }

    async fn read_response(
        &self,
        url: &Url,
        method: &str,
        headers: &[(String, String)],
    ) -> Result<moli_fetch::Response, LocalUrlError> {
        if method != "GET" {
            return Err(LocalUrlError::BlobMethod {
                method: method.to_owned(),
            });
        }
        let unavailable = || LocalUrlError::BlobUnavailable { url: url.clone() };
        let entry = self.inner.entry.as_ref().ok_or_else(unavailable)?;
        let ObjectUrlResourceData::Blob { data, mime_type } = &entry.data else {
            return Err(LocalUrlError::MediaSource);
        };
        let bytes = match data.shared_bytes() {
            Some(bytes) => bytes,
            None => {
                let (sender, receiver) = tokio::sync::oneshot::channel();
                data.read_all(Box::new(move |result| {
                    let _ = sender.send(result);
                }));
                Arc::from(
                    receiver
                        .await
                        .map_err(|_| unavailable())?
                        .map_err(|_| unavailable())?,
                )
            }
        };
        super::browser_response::blob_response(url, &bytes, mime_type, headers)
    }
}

fn blob_url_key(url: &Url) -> &str {
    url.as_str()
        .split_once('#')
        .map_or(url.as_str(), |(url, _)| url)
}

impl PartialEq for CapturedBlobResource {
    fn eq(&self, other: &Self) -> bool {
        if Arc::ptr_eq(&self.inner, &other.inner) {
            return true;
        }
        if self.inner.url != other.inner.url {
            return false;
        }
        // A Blob URL identifies one immutable store entry. Compare resolution
        // and authority, never the potentially large or lazy byte source.
        match (&self.inner.entry, &other.inner.entry) {
            (None, None) => true,
            (Some(left), Some(right)) if left.access_key == right.access_key => {
                match (&left.data, &right.data) {
                    (
                        ObjectUrlResourceData::Blob {
                            mime_type: left, ..
                        },
                        ObjectUrlResourceData::Blob {
                            mime_type: right, ..
                        },
                    ) => left == right,
                    (ObjectUrlResourceData::MediaSource, ObjectUrlResourceData::MediaSource) => {
                        true
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }
}

impl Eq for CapturedBlobResource {}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[derive(Debug, Default)]
    struct Source(AtomicUsize);

    impl moli_file_api::BlobSource for Source {
        fn read(&self, range: std::ops::Range<usize>, complete: moli_file_api::BlobReadCompletion) {
            self.0.fetch_add(1, Ordering::Relaxed);
            complete(Ok([0, 128, 255, 65][range].to_vec()));
        }
    }

    fn resource(data: ObjectUrlResourceData) -> CapturedBlobResource {
        CapturedBlobResource {
            inner: Arc::new(CapturedBlobResourceData {
                url: Url::parse("blob:https://example.test/captured").unwrap(),
                entry: Some(ObjectUrlResourceEntry {
                    data,
                    access_key: None,
                }),
            }),
        }
    }

    #[tokio::test]
    async fn lazy_blob_resources_preserve_method_range_mime_and_fragment_semantics() {
        fn require_send_sync<T: Send + Sync>() {}
        require_send_sync::<CapturedBlobResource>();
        let source = Arc::new(Source::default());
        let captured = resource(ObjectUrlResourceData::Blob {
            data: moli_file_api::BlobData::from_source(source.clone(), 4),
            mime_type: "application/example".to_owned(),
        });
        assert_eq!(captured, captured.clone());
        assert_eq!(source.0.load(Ordering::Relaxed), 0);
        let url = &captured.inner.url;
        for method in ["HEAD", "POST"] {
            let error = captured
                .response(url, method, &[])
                .await
                .unwrap()
                .unwrap_err();
            assert!(matches!(
                error.downcast_ref::<LocalUrlError>(),
                Some(LocalUrlError::BlobMethod { .. })
            ));
        }
        let mut different = url.clone();
        different.set_query(Some("query"));
        assert!(captured.response(&different, "GET", &[]).await.is_none());
        assert_eq!(source.0.load(Ordering::Relaxed), 0);
        let mut fragment = url.clone();
        fragment.set_fragment(Some("fragment"));
        let response = captured
            .response(
                &fragment,
                "GET",
                &[("Range".to_owned(), "bytes=1-2".to_owned())],
            )
            .await
            .unwrap()
            .unwrap();
        assert_eq!(response.status, 206);
        assert_eq!(response.body_bytes(), [128, 255]);
        assert!(
            response
                .headers
                .iter()
                .any(|(name, value)| name.eq_ignore_ascii_case("content-type")
                    && value == b"application/example")
        );
        assert_eq!(source.0.load(Ordering::Relaxed), 1);
    }

    #[tokio::test]
    async fn denied_or_unresolved_resources_fail_before_backing_io() {
        let source = Arc::new(Source::default());
        let captured = resource(ObjectUrlResourceData::Blob {
            data: moli_file_api::BlobData::from_source(source.clone(), 4),
            mime_type: String::new(),
        })
        .for_fetch_environment(None);
        let url = &captured.inner.url;
        let error = captured
            .response(url, "GET", &[])
            .await
            .unwrap()
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<LocalUrlError>(),
            Some(LocalUrlError::BlobUnavailable { .. })
        ));
        assert_eq!(source.0.load(Ordering::Relaxed), 0);
        let missing =
            CapturedBlobResource::capture(&Url::parse("blob:null/missing-resource").unwrap())
                .unwrap();
        assert!(missing.inner.entry.is_none());
        assert!(
            missing
                .response(&missing.inner.url, "GET", &[])
                .await
                .unwrap()
                .is_err()
        );
        assert!(CapturedBlobResource::capture_serialized("https://example.test/blob").is_none());
        assert!(CapturedBlobResource::capture_serialized("data:text/plain,blob").is_none());
    }

    #[tokio::test]
    async fn byte_resource_fetch_does_not_treat_media_source_as_a_blob() {
        let captured = resource(ObjectUrlResourceData::MediaSource);
        let error = captured
            .response(&captured.inner.url, "GET", &[])
            .await
            .unwrap()
            .unwrap_err();
        assert!(matches!(
            error.downcast_ref::<LocalUrlError>(),
            Some(LocalUrlError::MediaSource)
        ));
    }
}
