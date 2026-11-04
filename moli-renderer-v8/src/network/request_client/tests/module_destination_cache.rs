use super::*;
use anyhow::Context;
use tokio::task::JoinSet;

struct DestinationServer {
    url: String,
    tasks: JoinSet<Result<Vec<String>>>,
}

impl DestinationServer {
    async fn spawn(request_count: usize, hold_responses: bool) -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0").await?;
        let url = format!("http://{}/module", listener.local_addr()?);
        let mut tasks = JoinSet::new();
        tasks.spawn(async move {
            let mut destinations = Vec::new();
            let mut held = Vec::new();
            for _ in 0..request_count {
                let (mut stream, _) = listener.accept().await?;
                let request = read_http_request_text(&mut stream).await?;
                let header = |name: &str| request.lines().filter_map(|line| line.split_once(':'))
                    .find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.trim());
                assert_eq!(header("Accept"), Some("*/*"), "keep Accept identical across destinations");
                let destination = header("Sec-Fetch-Dest").context("fetch destination")?.to_owned();
                let body = format!("{destination} response");
                let response = format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nCache-Control: max-age=600\r\nVary: Sec-Fetch-Dest\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                destinations.push(destination);
                if hold_responses {
                    held.push((stream, response));
                } else {
                    stream.write_all(response.as_bytes()).await?;
                }
            }
            // Both destinations must reach the network before either response
            // can finish, deterministically exercising pending-load isolation.
            for (mut stream, response) in held {
                stream.write_all(response.as_bytes()).await?;
            }
            Ok(destinations)
        });
        Ok(Self { url, tasks })
    }

    async fn finish(&mut self) -> Result<Vec<String>> {
        self.tasks
            .join_next()
            .await
            .context("destination server task")?
            .context("destination server panicked")?
    }
}

fn destination_request(url: &str, metadata: BrowserRequestMetadata) -> Result<Request> {
    let url = Url::parse(url)?;
    Ok(Request::new_browser(
        "GET",
        url.clone(),
        None,
        vec![("Accept".to_owned(), "*/*".to_owned())],
        (&url).into(),
    )
    .with_page_network_policy()
    .with_initiator_url(&url.join("/page.html")?)
    .with_browser_request_metadata(metadata))
}

#[tokio::test(flavor = "multi_thread")]
async fn script_text_memory_cache_isolates_fetch_destinations_in_both_orders() -> Result<()> {
    for reverse in [false, true] {
        let mut destinations = [
            BrowserRequestMetadata::Script,
            BrowserRequestMetadata::TextModule,
            BrowserRequestMetadata::JsonModule,
            BrowserRequestMetadata::StyleModule,
        ];
        if reverse {
            destinations.reverse();
        }
        let mut server = DestinationServer::spawn(destinations.len(), false).await?;
        let loader = ResourceRequestClient::new(&FetchConfig::default())?;
        for metadata in destinations.iter().copied() {
            let request = destination_request(&server.url, metadata)?
                .with_script_fetch_metadata(ScriptFetchRequestMetadata::default());
            let first = timeout(
                Duration::from_secs(5),
                loader.fetch_cacheable_script_text_stream(request.clone()),
            )
            .await??;
            let cached = timeout(
                Duration::from_secs(5),
                loader.fetch_cacheable_script_text_stream(request),
            )
            .await??;
            let expected = format!("{} response", metadata.fetch_destination());
            assert_eq!(first.body_text(), expected);
            assert!(!first.from_cache, "{metadata:?}");
            assert_eq!(cached.body_text(), expected);
            assert_eq!(cached.cache_state, moli_fetch::ResponseCacheState::Local);
        }
        assert_eq!(
            server.finish().await?,
            destinations
                .iter()
                .map(|metadata| metadata.fetch_destination())
                .collect::<Vec<_>>()
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn script_text_pending_loads_coalesce_only_within_one_fetch_destination() -> Result<()> {
    let mut server = DestinationServer::spawn(2, true).await?;
    let loader = ResourceRequestClient::new(&FetchConfig::default())?;
    let script = destination_request(&server.url, BrowserRequestMetadata::Script)?
        .with_script_fetch_metadata(ScriptFetchRequestMetadata::default());
    let text = destination_request(&server.url, BrowserRequestMetadata::TextModule)?
        .with_script_fetch_metadata(ScriptFetchRequestMetadata::default());
    let responses = timeout(Duration::from_secs(5), async {
        tokio::try_join!(
            loader.fetch_cacheable_script_text_stream(script.clone()),
            loader.fetch_cacheable_script_text_stream(text.clone()),
            loader.fetch_cacheable_script_text_stream(script),
            loader.fetch_cacheable_script_text_stream(text),
        )
    })
    .await??;
    assert_eq!(responses.0.body_text(), "script response");
    assert_eq!(responses.1.body_text(), "text response");
    assert_eq!(responses.2.body_text(), "script response");
    assert_eq!(responses.3.body_text(), "text response");
    let mut destinations = server.finish().await?;
    destinations.sort();
    assert_eq!(destinations, ["script", "text"]);
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn raw_subresource_memory_cache_isolates_fetch_destinations_in_both_orders() -> Result<()> {
    for reverse in [false, true] {
        let mut destinations = [
            BrowserRequestMetadata::Fetch,
            BrowserRequestMetadata::TextModule,
            BrowserRequestMetadata::JsonModule,
            BrowserRequestMetadata::StyleModule,
            BrowserRequestMetadata::Manifest,
        ];
        if reverse {
            destinations.reverse();
        }
        let mut server = DestinationServer::spawn(destinations.len(), false).await?;
        let loader = ResourceRequestClient::new(&FetchConfig::default())?;
        for metadata in destinations.iter().copied() {
            let request = destination_request(&server.url, metadata)?;
            for cached in [false, true] {
                let response = timeout(Duration::from_secs(5), async {
                    loader
                        .fetch_raw_stream_with_cancel(request.clone(), FetchCancelHandle::new())
                        .await?
                        .into_materialized_raw_response()
                        .await
                })
                .await??;
                assert_eq!(
                    response.body_bytes(),
                    format!("{} response", metadata.fetch_destination()).as_bytes()
                );
                assert_eq!(
                    response.cache_state == moli_fetch::ResponseCacheState::Local,
                    cached,
                    "{metadata:?}"
                );
            }
        }
        assert_eq!(
            server.finish().await?,
            destinations
                .iter()
                .map(|metadata| metadata.fetch_destination())
                .collect::<Vec<_>>()
        );
    }
    Ok(())
}
