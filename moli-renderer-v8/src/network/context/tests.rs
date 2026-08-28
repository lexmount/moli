use moli_fetch::FetchConfig;
use url::Url;

use crate::network::{
    context::{WorkerResourceLoader, WorkerResourceLoaderState, WorkerResourceOwner},
    loads::{ResourceLoadDisposition, ResourceLoadKind},
};
use crate::{
    frame_owner_model::{DocumentId, FrameDocumentTaskOwner, FrameSchedulerLaneId, LocalWindowId},
    native_bridge::WindowDocumentOwner,
    network::ResourceRequestClient,
};

use super::{
    DocumentFetchContext, DocumentResourceLoader, DocumentResourceLoaderRegistry,
    DocumentResourceLoaderState,
};

fn owner(document_id: u64) -> FrameDocumentTaskOwner {
    FrameDocumentTaskOwner::new(
        FrameSchedulerLaneId(7),
        LocalWindowId(11),
        DocumentId(document_id),
    )
}

fn context(document_id: u64, url: &str) -> DocumentFetchContext {
    let url = Url::parse(url).expect("document URL");
    DocumentFetchContext::new(
        WindowDocumentOwner::Frame(owner(document_id)),
        url.clone(),
        url.clone(),
        moli_url::origin_ascii_serialization(&url),
    )
}

fn resource_task_runner() -> crate::network::RendererResourceTaskRunner {
    crate::network::RendererResourceTaskRunner::from_current_tokio()
        .expect("resource authority test must own a Tokio runtime")
}

fn document_loader(
    request_client: ResourceRequestClient,
    document_id: u64,
    url: &str,
) -> DocumentResourceLoader {
    DocumentResourceLoader::new(
        request_client,
        resource_task_runner(),
        context(document_id, url),
    )
}

#[test]
fn synthetic_document_context_preserves_its_inherited_origin() {
    let document_url = Url::parse("about:blank").expect("about:blank URL");
    let base_url = Url::parse("https://creator.test/base/").expect("creator base URL");
    let context = DocumentFetchContext::new(
        WindowDocumentOwner::Frame(owner(1)),
        document_url,
        base_url,
        "https://creator.test",
    );

    assert_eq!(context.origin(), "https://creator.test");
}

#[tokio::test(flavor = "current_thread")]
async fn document_authorities_share_backend_but_not_lifecycle() {
    let transport =
        ResourceRequestClient::new(&FetchConfig::default()).expect("resource transport");
    let task_runner = resource_task_runner();
    let first = DocumentResourceLoader::new(
        transport.clone(),
        task_runner.clone(),
        context(1, "https://example.test/a"),
    );
    let second = first.fork_for_document(context(2, "https://example.test/b"));

    assert_ne!(
        first.loader_id_for_diagnostics(),
        second.loader_id_for_diagnostics()
    );
    assert!(
        first
            .request_client()
            .shares_resource_runtime_with(second.request_client())
    );
    assert!(first.task_runner().shares_executor_with(&task_runner));
    assert!(
        second
            .task_runner()
            .shares_executor_with(&first.task_runner())
    );
    assert_eq!(first.state(), DocumentResourceLoaderState::Active);
    assert_eq!(second.state(), DocumentResourceLoaderState::Active);

    first.begin_detach();
    first.finish_detach();
    assert_eq!(first.state(), DocumentResourceLoaderState::Detached);
    assert_eq!(second.state(), DocumentResourceLoaderState::Active);
    assert!(second.accepts_ordinary_loads());
}

#[tokio::test(flavor = "current_thread")]
async fn inherited_child_document_loader_preserves_top_frame_site_context() {
    let transport =
        ResourceRequestClient::new(&FetchConfig::default()).expect("resource transport");
    let top = document_loader(transport.handle(), 1, "https://top.test/page");
    let child = top.fork_for_document(context(2, "https://frame.test/challenge"));

    let top_context = top
        .request_client()
        .browser_site_context()
        .expect("top Document browser-site context");
    let child_context = child
        .request_client()
        .browser_site_context()
        .expect("child Document browser-site context");

    assert_eq!(
        top_context.site_for_cookies_url.as_ref(),
        Some(&Url::parse("https://top.test/page").unwrap())
    );
    assert_eq!(
        child_context.site_for_cookies_url,
        top_context.site_for_cookies_url
    );
    assert_eq!(
        child_context.top_frame_origin_url,
        top_context.top_frame_origin_url
    );
}

#[tokio::test(flavor = "current_thread")]
async fn registry_rejects_retired_generation_without_hiding_current_generation() {
    let transport =
        ResourceRequestClient::new(&FetchConfig::default()).expect("resource transport");
    let first = document_loader(transport.handle(), 1, "https://example.test/one");
    let second = first.fork_for_document(context(2, "https://example.test/two"));
    let registry = DocumentResourceLoaderRegistry::default();
    registry.register(WindowDocumentOwner::Frame(owner(1)), first.clone());
    registry.register(WindowDocumentOwner::Frame(owner(2)), second.clone());

    let retired = registry
        .retire(WindowDocumentOwner::Frame(owner(1)))
        .expect("first authority");
    assert_eq!(retired.state(), DocumentResourceLoaderState::Detached);
    assert!(registry.get(WindowDocumentOwner::Frame(owner(1))).is_none());
    assert_eq!(
        registry
            .get(WindowDocumentOwner::Frame(owner(2)))
            .expect("current authority")
            .loader_id_for_diagnostics(),
        second.loader_id_for_diagnostics()
    );
    assert_eq!(registry.len(), 1);
}

#[tokio::test(flavor = "current_thread")]
async fn registry_replaces_future_transport_without_rebinding_existing_loads() {
    let original = ResourceRequestClient::new(&FetchConfig::default()).expect("original transport");
    original.set_extra_http_headers(&[("x-request-policy".to_owned(), "original".to_owned())]);
    let original_runtime = original.browser_resource_runtime();
    let authority = document_loader(original.handle(), 1, "https://example.test/one");
    let existing_load = authority
        .register_load(
            ResourceLoadKind::Fetch,
            ResourceLoadDisposition::Ordinary,
            None,
        )
        .expect("active original authority");
    let mut replacement_config = FetchConfig::default();
    replacement_config.set_user_agent("replacement-resource-client");
    let replacement =
        ResourceRequestClient::new(&replacement_config).expect("replacement resource transport");
    let replacement_runtime = replacement.browser_resource_runtime();
    let replacement_view = authority.with_replacement_transport(replacement.handle());
    replacement_view
        .request_client()
        .set_extra_http_headers(&[("x-request-policy".to_owned(), "replacement".to_owned())]);
    let registry = DocumentResourceLoaderRegistry::default();
    let document_owner = WindowDocumentOwner::Frame(owner(1));
    registry.register(document_owner, authority.clone());

    registry.replace_transport_view(document_owner, replacement_view);

    let installed = registry.get(document_owner).expect("installed authority");
    assert!(installed.shares_authority_with(&authority));
    assert_eq!(
        installed.loader_id_for_diagnostics(),
        authority.loader_id_for_diagnostics()
    );
    assert_eq!(
        installed.request_client().user_agent(),
        "replacement-resource-client"
    );
    let replacement_load = installed
        .register_load(
            ResourceLoadKind::Fetch,
            ResourceLoadDisposition::Ordinary,
            None,
        )
        .expect("active replacement transport view");
    assert!(
        existing_load
            .request_client()
            .browser_resource_runtime()
            .shares_state_with(&original_runtime)
    );
    assert!(
        !existing_load
            .request_client()
            .browser_resource_runtime()
            .shares_state_with(&replacement_runtime),
        "an in-flight request keeps the backend captured when it started"
    );
    assert!(
        replacement_load
            .request_client()
            .browser_resource_runtime()
            .shares_state_with(&replacement_runtime),
        "new requests use the replacement transport"
    );
    let existing_request = existing_load
        .request_client()
        .page_network_policy()
        .snapshot()
        .apply_to_request(
            moli_fetch::Request::get("https://example.test/existing")
                .expect("existing request")
                .with_page_network_policy(),
        )
        .expect("existing request policy");
    let replacement_request = replacement_load
        .request_client()
        .page_network_policy()
        .snapshot()
        .apply_to_request(
            moli_fetch::Request::get("https://example.test/replacement")
                .expect("replacement request")
                .with_page_network_policy(),
        )
        .expect("replacement request policy");
    assert_eq!(
        existing_request.request_headers,
        vec![("x-request-policy".to_owned(), "original".to_owned())],
        "an in-flight request keeps its prepared configuration"
    );
    assert_eq!(
        replacement_request.request_headers,
        vec![("x-request-policy".to_owned(), "replacement".to_owned())],
        "new requests use the replacement configuration"
    );

    installed.request_client().set_network_offline(true);
    assert!(existing_load.network_offline());
    assert!(replacement_load.network_offline());
}

#[tokio::test(flavor = "current_thread")]
#[should_panic(expected = "cannot replace transport for unknown Document")]
async fn registry_rejects_transport_replacement_without_registered_authority() {
    let request_client =
        ResourceRequestClient::new(&FetchConfig::default()).expect("resource transport");
    let replacement = document_loader(
        request_client.handle(),
        1,
        "https://example.test/replacement",
    );

    DocumentResourceLoaderRegistry::default()
        .replace_transport_view(WindowDocumentOwner::Frame(owner(1)), replacement);
}

#[tokio::test(flavor = "current_thread")]
#[should_panic(expected = "cannot be rebound to a second resource authority")]
async fn registry_rejects_transport_replacement_with_different_authority() {
    let request_client =
        ResourceRequestClient::new(&FetchConfig::default()).expect("resource transport");
    let registered = document_loader(request_client.clone(), 1, "https://example.test/registered");
    let replacement = document_loader(
        request_client.handle(),
        1,
        "https://example.test/replacement-authority",
    );
    let registry = DocumentResourceLoaderRegistry::default();
    let document_owner = WindowDocumentOwner::Frame(owner(1));
    registry.register(document_owner, registered);

    registry.replace_transport_view(document_owner, replacement);
}

#[tokio::test(flavor = "current_thread")]
async fn detached_document_can_spawn_network_only_keepalive_from_captured_context() {
    let transport =
        ResourceRequestClient::new(&FetchConfig::default()).expect("resource transport");
    let loader = document_loader(transport.handle(), 1, "https://example.test/source");
    let request_client = loader.frozen_request_client();
    let runtime = request_client.browser_resource_runtime();
    let cancel = moli_fetch::FetchCancelHandle::new();

    assert!(loader.begin_detach());
    loader.finish_detach();
    let load = loader
        .register_network_only_keepalive_load(
            ResourceLoadKind::CspReport,
            request_client,
            Some(cancel.clone()),
        )
        .expect("captured detached context should retain network-only authority");

    assert!(load.is_detached_keepalive());
    assert!(!cancel.is_cancelled());
    assert_eq!(
        runtime.detached_keepalive_diagnostics().active_load_count,
        1
    );
    load.finish();
    assert_eq!(
        runtime.detached_keepalive_diagnostics().active_load_count,
        0
    );
}

#[tokio::test(flavor = "current_thread")]
async fn worker_retirement_cancels_ordinary_and_transfers_keepalive_loads() {
    let transport =
        ResourceRequestClient::new(&FetchConfig::default()).expect("resource transport");
    let loader = WorkerResourceLoader::new(
        transport.handle(),
        WorkerResourceOwner::Dedicated {
            name: "owner".into(),
        },
        resource_task_runner(),
    );
    let ordinary_cancel = moli_fetch::FetchCancelHandle::new();
    let keepalive_cancel = moli_fetch::FetchCancelHandle::new();
    let ordinary = loader
        .register_load(
            ResourceLoadKind::Fetch,
            ResourceLoadDisposition::Ordinary,
            Some(ordinary_cancel.clone()),
        )
        .expect("active worker ordinary load");
    let keepalive = loader
        .register_load(
            ResourceLoadKind::Fetch,
            ResourceLoadDisposition::Keepalive,
            Some(keepalive_cancel.clone()),
        )
        .expect("active worker keepalive load");

    assert!(loader.begin_detach());
    loader.finish_detach();

    assert_eq!(loader.state(), WorkerResourceLoaderState::Detached);
    assert!(ordinary_cancel.is_cancelled());
    assert!(ordinary.is_cancelled());
    assert!(!keepalive_cancel.is_cancelled());
    assert!(keepalive.is_detached_keepalive());
    keepalive.finish();
}

#[tokio::test(flavor = "current_thread")]
async fn dropping_unpublished_worker_authority_cancels_ordinary_loads() {
    let transport =
        ResourceRequestClient::new(&FetchConfig::default()).expect("resource transport");
    let loader = WorkerResourceLoader::new(
        transport.handle(),
        WorkerResourceOwner::Dedicated {
            name: "bootstrap-failure".into(),
        },
        resource_task_runner(),
    );
    let cancel = moli_fetch::FetchCancelHandle::new();
    let load = loader
        .register_load(
            ResourceLoadKind::Script,
            ResourceLoadDisposition::Ordinary,
            Some(cancel.clone()),
        )
        .expect("active worker load");

    drop(loader);

    assert!(cancel.is_cancelled());
    assert!(load.is_cancelled());
}
