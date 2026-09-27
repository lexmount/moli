use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

struct DocumentMimeServer {
    origin: String,
    shutdown: Option<tokio::sync::oneshot::Sender<()>>,
    task: Option<tokio::task::JoinHandle<Result<()>>>,
}

impl DocumentMimeServer {
    async fn spawn() -> Result<Self> {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
        let origin = format!("http://{}", listener.local_addr()?);
        let (shutdown, mut shutting_down) = tokio::sync::oneshot::channel();
        let task = tokio::spawn(async move {
            let mut requests = tokio::task::JoinSet::new();
            loop {
                tokio::select! {
                    _ = &mut shutting_down => break,
                    accepted = listener.accept() => {
                        let (socket, _) = accepted?;
                        requests.spawn(serve_document(socket));
                    }
                    Some(result) = requests.join_next(), if !requests.is_empty() => { result??; }
                }
            }
            requests.shutdown().await;
            Ok(())
        });
        Ok(Self {
            origin,
            shutdown: Some(shutdown),
            task: Some(task),
        })
    }

    async fn shutdown(mut self) -> Result<()> {
        if let Some(shutdown) = self.shutdown.take() {
            let _ = shutdown.send(());
        }
        self.task.take().expect("server task").await?
    }
}

impl Drop for DocumentMimeServer {
    fn drop(&mut self) {
        if let Some(task) = self.task.take() {
            task.abort();
        }
    }
}

async fn serve_document(mut socket: tokio::net::TcpStream) -> Result<()> {
    let mut request = Vec::new();
    while !request.windows(4).any(|bytes| bytes == b"\r\n\r\n") {
        let mut buffer = [0; 4096];
        let count = socket.read(&mut buffer).await?;
        if count == 0 {
            return Ok(());
        }
        request.extend_from_slice(&buffer[..count]);
    }
    let request = String::from_utf8(request)?;
    let path = request
        .split_whitespace()
        .nth(1)
        .context("request target")?;
    let url = url::Url::parse(&format!("http://fixture{path}"))?;
    let mut body = b"<!doctype html><meta charset=utf-8><body>parent".to_vec();
    let mut fields = vec!["text/html;charset=utf-8".to_owned()];
    if url.path() == "/mime" {
        fields = url
            .query_pairs()
            .filter(|(key, _)| key == "value")
            .map(|(_, value)| value.into_owned())
            .collect();
        let encoding = url
            .query_pairs()
            .find(|(key, _)| key == "encoding")
            .map(|(_, value)| value.into_owned());
        body = match encoding.as_deref() {
            Some("gbk") => b"<b>\xbc\xd2\xbe\xd3</b>\n".to_vec(),
            Some("windows-1254") => b"<b>\xd0</b>\n".to_vec(),
            _ => b"<b>hi</b>\n".to_vec(),
        };
    }
    let mut head = format!(
        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\nConnection: close\r\n",
        body.len()
    );
    for field in fields {
        head.push_str(&format!("Content-Type: {field}\r\n"));
    }
    head.push_str("\r\n");
    socket.write_all(head.as_bytes()).await?;
    socket.write_all(&body).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn child_and_popup_documents_share_response_mime_and_charset_selection() -> Result<()> {
    let server = DocumentMimeServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let mut page = browser.fetch(&server.origin).await?;
    let observed = page
        .evaluate_runtime_expression_with_await_async(
            include_str!("../fixtures/document-response-mime.js"),
            true,
        )
        .await?;
    let observed: serde_json::Value = serde_json::from_str(
        observed["value"]
            .as_str()
            .context("document MIME probe result")?,
    )?;
    assert_eq!(observed["errors"], serde_json::json!([]));
    assert_eq!(observed["checked"], 64);
    server.shutdown().await
}

#[tokio::test(flavor = "multi_thread")]
async fn main_document_uses_extracted_mime_before_download_and_charset_selection() -> Result<()> {
    let server = DocumentMimeServer::spawn().await?;
    let browser = Browser::new(AppConfig::default())?;
    let cases: &[(&[&str], &str, &str)] = &[
        (
            &["application/octet-stream", "text/html;charset=gbk", "*/*"],
            "GBK",
            "家居",
        ),
        (
            &["text/plain;charset=utf-8", "text/html;charset=gbk"],
            "GBK",
            "家居",
        ),
        (
            &["text/html;charset=gbk", "text/html;charset=windows-1254"],
            "windows-1254",
            "Ğ",
        ),
        (&["text/html;charset=gbk", "text/html"], "GBK", "家居"),
    ];
    for (values, charset, text) in cases {
        for fields in [values.to_vec(), vec![&values.join(",")]] {
            let mut url = url::Url::parse(&format!("{}/mime", server.origin))?;
            for field in fields {
                url.query_pairs_mut().append_pair("value", field);
            }
            url.query_pairs_mut()
                .append_pair("encoding", &charset.to_ascii_lowercase());
            let mut page = browser.fetch(url.as_str()).await?;
            let observed = page.evaluate_runtime_expression_with_await_async(
                "JSON.stringify([document.contentType, document.characterSet, document.body.firstChild.localName, document.body.firstChild.textContent])", false,
            ).await?;
            let observed: serde_json::Value = serde_json::from_str(
                observed["value"]
                    .as_str()
                    .context("main document MIME result")?,
            )?;
            assert_eq!(
                observed,
                serde_json::json!(["text/html", charset, "b", text]),
                "{url}"
            );
        }
    }
    server.shutdown().await
}
