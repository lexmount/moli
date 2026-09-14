use super::*;
use crate::runtime::{RendererNetworkOutputItem, RendererWorkerNetworkReporter};
use crate::worker::{WorkerNetworkObserver, WorkerToParentMessage};
use moli_page_types::{ScriptNetworkOutputItem, SubresourceBodyFinishedResult};

#[test]
fn cancelled_worker_request_rejects_late_transport_delivery() {
    for transport_failed in [false, true] {
        let source = RendererWorkerNetworkReporter::unobserved_for_test();
        let (send, mut receive) = mpsc::unbounded_channel();
        let observer = WorkerNetworkObserver::Channel(send.downgrade());
        let url = Url::parse("data:text/plain,late").unwrap();
        let network = ResourceTransfer::for_worker(&source, observer, |network| {
            worker_request_started(
                network,
                &url,
                &url,
                "GET",
                &moli_fetch::RequestHeaders::default(),
                &None,
                SubresourceResourceType::Xhr,
            )
        })
        .unwrap();
        let delivery = WorkerRequestDelivery {
            network: network.clone(),
            completion: Some(Box::new(WorkerRequestCompletion {
                id: 1,
                network_request_headers: None,
                result: if transport_failed {
                    Err("late transport failure".to_owned().into())
                } else {
                    Ok(ResourceBodyResponse::from(
                        crate::network_host::local_url_response(&url).unwrap(),
                    ))
                },
            })),
        };
        // A timeout consumes the pending request before the transport packet
        // reaches its closed completion route. Its Drop must not finish twice.
        publish_worker_request_failure(&network, "timeout".to_owned().into());
        drop(delivery);
        let mut items = Vec::new();
        while let Ok(WorkerToParentMessage::Network(observation)) = receive.try_recv() {
            let RendererNetworkOutputItem::Resource(item) = observation.item() else {
                panic!("resource receipt")
            };
            items.push(item.clone());
        }
        assert_eq!(
            items.len(),
            2,
            "a retired request cannot publish a later response or terminal"
        );
        let ScriptNetworkOutputItem::SubresourceBodyFinished(terminal) = items[1].as_ref() else {
            panic!("one terminal after admission")
        };
        assert!(
            matches!(terminal.result(), SubresourceBodyFinishedResult::Failed(message) if message == "timeout")
        );
    }
}
