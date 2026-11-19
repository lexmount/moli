use moli_fetch::RequestMode;
use moli_fetch_types::{FetchResponseMetadata, FetchResponseType};
use moli_url::WebOrigin;

pub(crate) fn fetch_response_metadata(
    origin: &WebOrigin,
    head: &moli_fetch::ResponseHead,
    mode: RequestMode,
    delivery: Option<&crate::types::AsyncSubresourceFetchResponseFilter>,
    web_security_disabled: bool,
) -> FetchResponseMetadata {
    if let Some(metadata) = delivery.and_then(|delivery| delivery.service_worker_metadata()) {
        let mut metadata = metadata.clone();
        if metadata.response_type == FetchResponseType::Default {
            metadata.response_type = FetchResponseType::Basic;
        }
        if metadata.url_list.is_empty() {
            metadata.url_list = head.url_list().to_vec();
        }
        metadata.redirected |= head.redirected;
        return metadata;
    }
    let mut metadata = head.fetch_metadata_with_web_security(origin, mode, web_security_disabled);
    if let Some(delivery) = delivery {
        metadata.response_type = delivery.response_type();
    }
    metadata
}
