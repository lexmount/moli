use super::*;
use tokio::io::{AsyncReadExt, AsyncWriteExt};

async fn serve(mut socket: tokio::net::TcpStream) -> Result<()> {
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
    let parameter = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    let mut extra = String::new();
    let (mime, body) = match url.path() {
        "/encoding-parent" => {
            let charset = parameter("charset").unwrap_or_else(|| "windows-1253".to_owned());
            let source = "<!doctype html><body><script src='/encoding-parent.js'></script>";
            let body = match charset.as_str() {
                "UTF-16LE" => source.encode_utf16().flat_map(u16::to_le_bytes).collect(),
                "UTF-16BE" => source.encode_utf16().flat_map(u16::to_be_bytes).collect(),
                _ => source.as_bytes().to_vec(),
            };
            (format!("text/html;charset={charset}"), body)
        }
        "/encoding-parent.js" => (
            "text/javascript;charset=utf-8".to_owned(),
            include_bytes!("../fixtures/document-encoding-parent.js").to_vec(),
        ),
        "/encoding-child" => {
            let mime = parameter("header").map_or_else(
                || "text/html".to_owned(),
                |charset| format!("text/html;charset={charset}"),
            );
            let meta = parameter("meta")
                .map_or_else(String::new, |charset| format!("<meta charset='{charset}'>"));
            let token = serde_json::to_string(&parameter("token").unwrap_or_default())?;
            let report = format!(
                "<script>addEventListener('load',()=>{{const target=opener?opener.top:top;target.postMessage({{token:{token},charset:document.characterSet,text:document.querySelector('p').textContent}},'*')}})</script>"
            );
            let mut body = format!("<!doctype html>{meta}<body><p>").into_bytes();
            body.push(0xa2);
            body.extend_from_slice(format!("</p>{report}").as_bytes());
            if parameter("bom").as_deref() == Some("utf8") {
                body.splice(0..0, [0xef, 0xbb, 0xbf]);
            }
            if let Some(csp) = parameter("csp") {
                extra.push_str(&format!("Content-Security-Policy: {csp}\r\n"));
            }
            (mime, body)
        }
        "/redirect" => {
            extra.push_str(&format!(
                "Location: {}\r\n",
                parameter("to").context("redirect target")?
            ));
            ("text/html".to_owned(), Vec::new())
        }
        _ => (
            "text/html;charset=utf-8".to_owned(),
            b"<!doctype html><body>root".to_vec(),
        ),
    };
    let status = if url.path() == "/redirect" {
        "302 Found"
    } else {
        "200 OK"
    };
    let head = format!(
        "HTTP/1.1 {status}\r\nContent-Type: {mime}\r\nContent-Length: {}\r\nCache-Control: no-store\r\n{extra}Connection: close\r\n\r\n",
        body.len()
    );
    socket.write_all(head.as_bytes()).await?;
    socket.write_all(&body).await?;
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn document_encoding_inherits_only_from_the_eligible_container() -> Result<()> {
    let local = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let remote = tokio::net::TcpListener::bind("127.0.0.1:0").await?;
    let origins = [
        format!("http://{}", local.local_addr()?),
        format!("http://{}", remote.local_addr()?),
    ];
    let (shutdown, mut shutting_down) = tokio::sync::oneshot::channel::<()>();
    let server = tokio::spawn(async move {
        let mut requests = tokio::task::JoinSet::new();
        loop {
            tokio::select! {
                _ = &mut shutting_down => break,
                accepted = async { tokio::select! {
                    accepted = local.accept() => accepted,
                    accepted = remote.accept() => accepted,
                }} => {
                    let (socket, _) = accepted?;
                    requests.spawn(serve(socket));
                }
                Some(result) = requests.join_next(), if !requests.is_empty() => { result??; }
            }
        }
        requests.shutdown().await;
        Ok::<_, anyhow::Error>(())
    });
    let result = tokio::time::timeout(Duration::from_secs(30), async {
        let browser = Browser::new(AppConfig::default())?;
        let mut page = browser.fetch(&origins[0]).await?;
        let script = format!(
            "(globalThis.encodingOrigins={}, {})",
            serde_json::to_string(&origins)?,
            include_str!("../fixtures/document-encoding-inheritance.js")
        );
        let result = page
            .evaluate_runtime_expression_with_await_async(&script, true)
            .await?;
        let observed: serde_json::Value =
            serde_json::from_str(result["value"].as_str().context("encoding result")?)?;
        assert_eq!(observed["errors"], serde_json::json!([]));
        assert_eq!(observed["observations"].as_array().unwrap().len(), 27);
        Ok::<_, anyhow::Error>(())
    })
    .await;
    let _ = shutdown.send(());
    server.await??;
    result?
}
