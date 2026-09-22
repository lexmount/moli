use std::collections::HashMap;

use moli_core::page::{
    SubresourceNetworkOutcome, SubresourceNetworkRecord, SubresourceResourceType,
};
use moli_web_mime::effective_response_mime_essence;
use serde_json::{Value, json};
use url::Url;

#[derive(Debug)]
struct FrameResourceSnapshot {
    frame_id: Option<String>,
    url: String,
    payload: Value,
}

pub(super) fn attach_frame_resources(
    mut frame_tree: Value,
    records: &[SubresourceNetworkRecord],
) -> Value {
    let resources = frame_resource_snapshots(records);
    attach_resources_to_frame(&mut frame_tree, &resources, true);
    frame_tree
}

fn frame_resource_snapshots(records: &[SubresourceNetworkRecord]) -> Vec<FrameResourceSnapshot> {
    let mut snapshots = Vec::new();
    let mut index_by_resource = HashMap::new();

    for record in records {
        let Some(snapshot) = frame_resource_snapshot(record) else {
            continue;
        };
        let key = (snapshot.frame_id.clone(), snapshot.url.clone());
        if let Some(index) = index_by_resource.get(&key).copied() {
            snapshots[index] = snapshot;
        } else {
            index_by_resource.insert(key, snapshots.len());
            snapshots.push(snapshot);
        }
    }

    snapshots
}

fn frame_resource_snapshot(record: &SubresourceNetworkRecord) -> Option<FrameResourceSnapshot> {
    if !resource_type_appears_in_frame_tree(record.resource_type()) {
        return None;
    }

    let (url, mime_type, content_size, failed) = match record.outcome() {
        SubresourceNetworkOutcome::Success {
            final_url,
            response_headers,
            response_body,
            ..
        } => (
            url_without_fragment(final_url),
            effective_response_mime_essence(response_headers, None).unwrap_or_default(),
            response_body.len(),
            false,
        ),
        SubresourceNetworkOutcome::Failure { .. } => {
            (url_without_fragment(record.url()), String::new(), 0, true)
        }
    };

    let mut payload = json!({
        "url": url,
        "type": record.resource_type().as_cdp_type(),
        "mimeType": mime_type,
        "contentSize": content_size,
    });
    if failed {
        payload["failed"] = json!(true);
    }

    Some(FrameResourceSnapshot {
        frame_id: record.frame_id().map(str::to_owned),
        url,
        payload,
    })
}

fn resource_type_appears_in_frame_tree(resource_type: SubresourceResourceType) -> bool {
    !matches!(
        resource_type,
        SubresourceResourceType::Fetch
            | SubresourceResourceType::EventSource
            | SubresourceResourceType::Xhr
            | SubresourceResourceType::Ping
            | SubresourceResourceType::CspReport
            | SubresourceResourceType::WebSocket
    )
}

fn url_without_fragment(url: &Url) -> String {
    let mut url = url.clone();
    url.set_fragment(None);
    url.into()
}

fn attach_resources_to_frame(
    frame_tree: &mut Value,
    resources: &[FrameResourceSnapshot],
    is_root: bool,
) {
    let frame_id = frame_tree["frame"]["id"].as_str().unwrap_or_default();
    frame_tree["resources"] = Value::Array(
        resources
            .iter()
            .filter(|resource| {
                resource.frame_id.as_deref() == Some(frame_id)
                    || (is_root && resource.frame_id.is_none())
            })
            .map(|resource| resource.payload.clone())
            .collect(),
    );

    let Some(child_frames) = frame_tree
        .get_mut("childFrames")
        .and_then(Value::as_array_mut)
    else {
        return;
    };
    for child_frame in child_frames {
        attach_resources_to_frame(child_frame, resources, false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn successful_record(
        frame_id: Option<&str>,
        requested_url: &str,
        final_url: &str,
        resource_type: SubresourceResourceType,
        content_type: &str,
        body: &str,
    ) -> SubresourceNetworkRecord {
        SubresourceNetworkRecord::success(
            frame_id.map(str::to_owned),
            Url::parse("https://example.test/document").unwrap(),
            Url::parse(requested_url).unwrap(),
            "GET".to_owned(),
            Vec::new().into(),
            None,
            resource_type,
            None,
            Vec::new(),
            Url::parse(final_url).unwrap(),
            200,
            vec![("Content-Type".to_owned(), content_type.as_bytes().to_vec())],
            body.to_owned(),
            Vec::new(),
        )
    }

    #[test]
    fn resource_tree_uses_observed_final_response_metadata() {
        let records = vec![successful_record(
            None,
            "https://example.test/original.js",
            "https://cdn.example.test/app.js#cache-key",
            SubresourceResourceType::Script,
            "application/javascript; charset=utf-8",
            "window.loaded = true;",
        )];

        let tree = attach_frame_resources(
            json!({"frame": {"id": "ROOT"}, "childFrames": []}),
            &records,
        );

        assert_eq!(
            tree["resources"],
            json!([{
                "url": "https://cdn.example.test/app.js",
                "type": "Script",
                "mimeType": "application/javascript",
                "contentSize": 21,
            }])
        );
    }

    #[test]
    fn resource_tree_attributes_children_and_omits_raw_resources() {
        let records = vec![
            successful_record(
                Some("CHILD"),
                "https://example.test/child.css",
                "https://example.test/child.css",
                SubresourceResourceType::Stylesheet,
                "text/css",
                "p{}",
            ),
            successful_record(
                None,
                "https://example.test/data",
                "https://example.test/data",
                SubresourceResourceType::Fetch,
                "application/json",
                "{}",
            ),
            successful_record(
                Some("DETACHED"),
                "https://example.test/stale.js",
                "https://example.test/stale.js",
                SubresourceResourceType::Script,
                "text/javascript",
                "0",
            ),
        ];

        let tree = attach_frame_resources(
            json!({
                "frame": {"id": "ROOT"},
                "childFrames": [{"frame": {"id": "CHILD"}}],
            }),
            &records,
        );

        assert_eq!(tree["resources"], json!([]));
        assert_eq!(
            tree["childFrames"][0]["resources"],
            json!([{
                "url": "https://example.test/child.css",
                "type": "Stylesheet",
                "mimeType": "text/css",
                "contentSize": 3,
            }])
        );
    }

    #[test]
    fn resource_tree_replaces_duplicate_cache_entries_and_marks_failures() {
        let failed = SubresourceNetworkRecord::failure(
            None,
            Url::parse("https://example.test/document").unwrap(),
            Url::parse("https://example.test/app.js#fragment").unwrap(),
            "GET".to_owned(),
            Vec::new().into(),
            None,
            SubresourceResourceType::Script,
            "connection reset".to_owned(),
        );
        let successful = successful_record(
            None,
            "https://example.test/app.js",
            "https://example.test/app.js",
            SubresourceResourceType::Script,
            "text/javascript",
            "ok",
        );

        let failed_tree = attach_frame_resources(
            json!({"frame": {"id": "ROOT"}}),
            std::slice::from_ref(&failed),
        );
        assert_eq!(failed_tree["resources"][0]["failed"], json!(true));
        assert_eq!(failed_tree["resources"][0]["contentSize"], json!(0));

        let recovered_tree =
            attach_frame_resources(json!({"frame": {"id": "ROOT"}}), &[failed, successful]);
        assert_eq!(recovered_tree["resources"].as_array().unwrap().len(), 1);
        assert_eq!(
            recovered_tree["resources"][0],
            json!({
                "url": "https://example.test/app.js",
                "type": "Script",
                "mimeType": "text/javascript",
                "contentSize": 2,
            })
        );
    }
}
