use super::*;
use std::{
    ffi::{c_char, c_int, c_void},
    io::Write,
};

extern "C" fn count_debug_calls(
    _easy: *mut curl_sys::CURL,
    _kind: curl_sys::curl_infotype,
    _data: *mut c_char,
    _len: usize,
    counter: *mut c_void,
) -> c_int {
    // SAFETY: tests retain the boxed counter until after Easy2 is dropped, and
    // only read it after perform has returned (or its thread has been joined).
    unsafe { *counter.cast::<usize>() += 1 };
    0
}

fn install_debug_counter(easy: &mut Easy2<FetchTransferHandler>, counter: &mut usize) {
    let callback: curl_sys::curl_debug_callback = count_debug_calls;
    // SAFETY: callback has curl_debug_callback's ABI; each caller retains the
    // counter allocation for the full lifetime of this easy handle.
    unsafe {
        assert_eq!(
            curl_sys::curl_easy_setopt(easy.raw(), curl_sys::CURLOPT_DEBUGFUNCTION, callback),
            curl_sys::CURLE_OK
        );
        assert_eq!(
            curl_sys::curl_easy_setopt(
                easy.raw(),
                curl_sys::CURLOPT_DEBUGDATA,
                std::ptr::from_mut(counter).cast::<c_void>(),
            ),
            curl_sys::CURLE_OK
        );
    }
}

fn read_head(stream: &mut std::net::TcpStream) -> std::io::Result<Vec<u8>> {
    stream.set_read_timeout(Some(Duration::from_secs(5)))?;
    let mut head = Vec::new();
    let mut byte = [0];
    while !head.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte)?;
        head.extend_from_slice(&byte);
    }
    Ok(head)
}

#[test]
fn prerequest_timing_survives_moves_and_reuse_without_verbose() -> Result<()> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let request = Request::get(&format!("http://{}/timing", listener.local_addr()?))?;
    let server = thread::spawn(move || -> std::io::Result<()> {
        // All three requests must reuse one connection, including after reset.
        let (mut stream, _) = listener.accept()?;
        for method in ["GET", "HEAD", "POST"] {
            let head = read_head(&mut stream)?;
            assert!(head.starts_with(format!("{method} ").as_bytes()));
            if method == "POST" {
                let mut body = [0; 5];
                stream.read_exact(&mut body)?;
                assert_eq!(&body, b"probe");
            }
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\n\r\n")?;
        }
        Ok(())
    });
    let cancel = FetchCancelHandle::new();
    let mut debug_calls = Box::new(0);
    let mut easy = Easy2::new(FetchTransferHandler::new_buffered(ResponseCollector::new(
        Some(cancel.clone()),
    )));
    let mut previous = Vec::new();
    for method in ["GET", "HEAD", "POST"] {
        easy.reset();
        let before = Instant::now();
        cancel.begin_response_progress();
        let observer = FetchCancelHandle::new();
        observer.adopt_response_progress(&cancel);
        easy.get_mut().buffered_mut().unwrap().begin_request(None);
        configure_network_observation(&mut easy, &request, None, false)?;
        install_debug_counter(&mut easy, &mut debug_calls);
        easy.url(request.url.as_str())?;
        easy.proxy("")?;
        easy.timeout(Duration::from_secs(5))?;
        if method == "HEAD" {
            easy.nobody(true)?;
        } else if method == "POST" {
            easy.post_fields_copy(b"probe")?;
        }
        // A newer selected token must not redirect callbacks on this easy.
        cancel.begin_response_progress();
        // Moving Easy2 to another thread must leave PREREQDATA valid.
        easy = thread::spawn(move || {
            easy.perform().unwrap();
            easy
        })
        .join()
        .unwrap();
        let timing = observer.response_timing();
        assert!(timing.request_started_at().unwrap() >= before);
        assert!(timing.request_started_at().unwrap() <= timing.response_started_at().unwrap());
        assert!(cancel.response_timing().request_started_at().is_none());
        assert_eq!(*debug_calls, 0, "ordinary requests must not enable verbose");
        previous.push((observer, timing));
        for (observer, snapshot) in &previous {
            assert_eq!(&observer.response_timing(), snapshot);
        }
    }
    drop(easy);
    server.join().unwrap()?;
    Ok(())
}

#[test]
fn verbose_remains_enabled_for_header_observation_and_proxy_diagnostics() -> Result<()> {
    for capture_proxy in [false, true] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let mut request = Request::get(&format!("http://{}/verbose", listener.local_addr()?))?;
        if !capture_proxy {
            request =
                request.with_network_observation_recorder(NetworkObservationRecorder::default());
        }
        let server = thread::spawn(move || -> std::io::Result<()> {
            let (mut stream, _) = listener.accept()?;
            read_head(&mut stream)?;
            stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
        });
        let cancel = FetchCancelHandle::new();
        let mut debug_calls = Box::new(0);
        let mut easy = Easy2::new(FetchTransferHandler::new_buffered(ResponseCollector::new(
            Some(cancel.clone()),
        )));
        configure_network_observation(&mut easy, &request, None, capture_proxy)?;
        install_debug_counter(&mut easy, &mut debug_calls);
        easy.url(request.url.as_str())?;
        easy.proxy("")?;
        easy.timeout(Duration::from_secs(5))?;
        easy.perform()?;
        assert!(*debug_calls > 0);
        assert!(cancel.response_timing().request_started_at().is_some());
        drop(easy);
        server.join().unwrap()?;
    }
    Ok(())
}

#[test]
fn failed_proxy_connect_does_not_start_the_origin_request() -> Result<()> {
    let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
    let proxy = format!("http://{}", listener.local_addr()?);
    let server = thread::spawn(move || -> std::io::Result<()> {
        let (mut stream, _) = listener.accept()?;
        assert!(read_head(&mut stream)?.starts_with(b"CONNECT "));
        stream
            .write_all(b"HTTP/1.1 403 Forbidden\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
    });
    let request = Request::get("https://origin.invalid/timing")?;
    let cancel = FetchCancelHandle::new();
    let mut easy = Easy2::new(FetchTransferHandler::new_buffered(ResponseCollector::new(
        Some(cancel.clone()),
    )));
    configure_network_observation(&mut easy, &request, None, true)?;
    easy.url(request.url.as_str())?;
    easy.proxy(&proxy)?;
    easy.noproxy("")?;
    easy.timeout(Duration::from_secs(5))?;
    assert!(easy.perform().is_err());
    assert!(cancel.response_timing().request_started_at().is_none());
    assert_eq!(
        take_failed_proxy_connect_response(&mut easy)
            .unwrap()
            .status,
        403
    );
    drop(easy);
    server.join().unwrap()?;
    Ok(())
}
