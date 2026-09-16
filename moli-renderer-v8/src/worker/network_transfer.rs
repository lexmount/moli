use super::WorkerNetworkObserver;
use crate::{
    network::ResourceTransfer,
    runtime::{RendererNetworkRequest, RendererWorkerNetworkReporter},
};
use moli_page_types::SubresourceRequestStarted;
use std::sync::Arc;

impl ResourceTransfer {
    pub(crate) fn for_worker(
        source: &RendererWorkerNetworkReporter,
        observer: WorkerNetworkObserver,
        request: impl FnOnce(&RendererNetworkRequest) -> SubresourceRequestStarted,
    ) -> Option<Arc<Self>> {
        let network = source.start_request()?;
        let subsequent = observer.clone();
        let (transfer, started) =
            Self::start(network, move |event| subsequent.publish(event), request);
        observer.publish(started);
        Some(transfer)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::network::{ResourceResponseFailure, ResourceResponseHead, ResourceResponseObserver};
    use crate::worker::global_scope::worker_request_started;
    use moli_page_types::{ScriptNetworkOutputItem, SubresourceResponseBody};

    #[test]
    fn worker_resource_terminal_is_single_use_with_or_without_streaming() {
        use crate::runtime::RendererNetworkOutputItem;
        use crate::worker::WorkerToParentMessage;
        use moli_page_types::SubresourceBodyFinishedResult;

        for streamed in [false, true] {
            for failed in [false, true] {
                let source = RendererWorkerNetworkReporter::unobserved_for_test();
                let (send, mut receive) = tokio::sync::mpsc::unbounded_channel();
                let url = url::Url::parse("data:text/javascript,//ok").unwrap();
                let response = crate::network_host::local_url_response(&url).unwrap();
                let head = Arc::new(ResourceResponseHead {
                    status_text: None,
                    head: response.head(),
                    network_request_headers: None,
                });
                let transfer = ResourceTransfer::for_worker(
                    &source,
                    WorkerNetworkObserver::Channel(send.downgrade()),
                    |network| {
                        worker_request_started(
                            network,
                            &url,
                            &url,
                            "GET",
                            &moli_fetch::RequestHeaders::default(),
                            &None,
                            moli_page_types::SubresourceResourceType::Script,
                        )
                    },
                )
                .unwrap();
                let handle = transfer.handle();
                if streamed {
                    transfer.response_started(head.clone());
                    transfer.data_received(2);
                    if !failed {
                        transfer.data_received(2);
                    }
                }
                if failed {
                    transfer.failed(&ResourceResponseFailure::PartialBody {
                        message: "truncated".into(),
                        response: head.clone(),
                        body: SubresourceResponseBody::from_bytes(b"//".to_vec()),
                    });
                } else {
                    transfer.response_completed(&response);
                }
                assert_eq!(
                    transfer.handle(),
                    handle,
                    "completion preserves request identity"
                );
                assert!(
                    transfer.request().is_none(),
                    "completion releases continuation permission"
                );
                // Late callbacks and the final lease drop cannot duplicate or
                // change the committed terminal result.
                transfer.failed(&ResourceResponseFailure::Request("late".into()));
                transfer.response_completed(&response);
                transfer.response_started(head);
                transfer.data_received(2);
                drop(transfer);
                let mut items = Vec::new();
                while let Ok(WorkerToParentMessage::Network(observation)) = receive.try_recv() {
                    let RendererNetworkOutputItem::Resource(item) = observation.item() else {
                        panic!("script producer must publish resource facts")
                    };
                    items.push(item.clone());
                }
                assert_eq!(
                    items.len(),
                    if streamed {
                        if failed { 4 } else { 5 }
                    } else {
                        3
                    }
                );
                let ScriptNetworkOutputItem::SubresourceRequestStarted(start) = items[0].as_ref()
                else {
                    panic!("start first")
                };
                let ScriptNetworkOutputItem::SubresourceResponseStarted(head) = items[1].as_ref()
                else {
                    panic!("one real response head before terminal")
                };
                assert_eq!(head.handle(), start.handle());
                let ScriptNetworkOutputItem::SubresourceBodyFinished(terminal) =
                    items.last().unwrap().as_ref()
                else {
                    panic!("terminal last")
                };
                assert_eq!(terminal.handle(), start.handle());
                match (failed, terminal.result()) {
                    (false, SubresourceBodyFinishedResult::Ready(body)) => {
                        assert_eq!(body.clone_body_bytes(), b"//ok");
                        assert_eq!(terminal.data_was_streamed(), streamed);
                    }
                    (
                        true,
                        SubresourceBodyFinishedResult::FailedWithPartialBody {
                            error_text,
                            partial_body,
                        },
                    ) => {
                        assert_eq!(error_text, "truncated");
                        assert_eq!(partial_body.clone_body_bytes(), b"//");
                    }
                    result => panic!("the first terminal outcome must win: {result:?}"),
                }
            }
        }
    }
}
