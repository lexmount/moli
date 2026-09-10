//! Fixed HTTP/1.1 keepalive load alongside the WebSocket owner probe.
use anyhow::{Context, Result, ensure};
use curl::easy::{Easy2, Handler, WriteError};
use moli_curl::{
    CurlDnsResolution, CurlHttpSender, CurlMultiCompletion, CurlMultiJob, CurlMultiRuntime,
    CurlMultiRuntimeConfig,
};
use std::{
    io::{Read, Write},
    net::TcpListener,
    thread,
    time::{Duration, Instant},
};

#[derive(Debug, Default)]
pub struct Body(Vec<u8>);
impl Handler for Body {
    fn write(&mut self, data: &[u8]) -> Result<usize, WriteError> {
        self.0.extend_from_slice(data);
        Ok(data.len())
    }
}

pub struct Workload {
    pub runtime: CurlMultiRuntime<Body, ()>,
    completed: crossbeam_channel::Receiver<CurlMultiCompletion<Body, ()>>,
    url: String,
    peer: thread::JoinHandle<Result<()>>,
}

impl Workload {
    pub fn new() -> Result<Self> {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let url = format!("http://{}/probe", listener.local_addr()?);
        let peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(Duration::from_secs(30)))?;
            let mut header = Vec::new();
            loop {
                let mut byte = [0];
                if stream.read(&mut byte)? == 0 {
                    break;
                }
                header.push(byte[0]);
                if header.ends_with(b"\r\n\r\n") {
                    stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nok")?;
                    header.clear();
                }
            }
            Ok(())
        });
        let (runtime, completed) = CurlMultiRuntime::new(CurlMultiRuntimeConfig {
            thread_name: "moli-curl-http".to_owned(),
            ..CurlMultiRuntimeConfig::default()
        })?;
        let workload = Self {
            runtime,
            completed,
            url,
            peer,
        };
        for _ in 0..16 {
            request(
                &workload.runtime.http_sender(),
                &workload.completed,
                &workload.url,
            )?;
        }
        Ok(workload)
    }

    pub fn start(&self, end: Instant) -> thread::JoinHandle<Result<Vec<f64>>> {
        let sender = self.runtime.http_sender();
        let url = self.url.clone();
        let completed = self.completed.clone();
        thread::spawn(move || {
            let mut latency = Vec::new();
            while Instant::now() < end {
                let started = Instant::now();
                request(&sender, &completed, &url)?;
                latency.push(started.elapsed().as_secs_f64() * 1000.0);
                // A fixed 100 requests/s offered load; this is a benchmark,
                // not synchronization for a correctness test.
                thread::sleep(Duration::from_millis(10).saturating_sub(started.elapsed()));
            }
            Ok(latency)
        })
    }

    pub fn finish(self) -> Result<()> {
        drop(self.runtime);
        self.peer
            .join()
            .map_err(|_| anyhow::anyhow!("HTTP peer panicked"))?
    }
}

fn request(
    sender: &CurlHttpSender<Body, ()>,
    completed: &crossbeam_channel::Receiver<CurlMultiCompletion<Body, ()>>,
    url: &str,
) -> Result<()> {
    let mut easy = Easy2::new(Body::default());
    easy.url(url)?;
    easy.proxy("")?;
    sender
        .submit(CurlMultiJob {
            easy,
            context: (),
            origin: None,
            deadline: Some(Instant::now() + Duration::from_secs(10)),
            dns_resolution: CurlDnsResolution::curl_managed(),
            priority: 1,
            label: "owner probe HTTP".into(),
        })
        .map_err(|error| error.error)?;
    let completion = completed.recv_timeout(Duration::from_secs(10))?;
    completion.result?;
    ensure!(
        completion.easy.context("HTTP easy missing")?.get_ref().0 == b"ok",
        "HTTP body mismatch"
    );
    Ok(())
}

pub fn report(task: thread::JoinHandle<Result<Vec<f64>>>) -> Result<()> {
    let mut samples = task
        .join()
        .map_err(|_| anyhow::anyhow!("HTTP producer panicked"))??;
    samples.sort_by(f64::total_cmp);
    ensure!(!samples.is_empty(), "no HTTP samples");
    println!(
        "http_version=1.1 http_requests={} http_p50_ms={:.3} http_p95_ms={:.3}",
        samples.len(),
        samples[(samples.len() - 1) / 2],
        samples[(samples.len() - 1) * 95 / 100]
    );
    Ok(())
}
