#![cfg(target_env = "ohos")]

use std::{
    ffi::{CStr, c_char, c_void},
    path::PathBuf,
    sync::Arc,
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

use moli_core::page::ViewportOutput;
use parking_lot::{Condvar, Mutex};
use tokio::sync::mpsc;

mod browser;
#[path = "../../molio/src/downloads.rs"]
mod downloads;
mod model;
mod tab;

use model::{BrowserState, Command};

#[derive(Default)]
pub(crate) struct Shared {
    pub state: Mutex<BrowserState>,
    pub viewport: Mutex<Option<Arc<ViewportOutput>>>,
    frame_generation: Mutex<u64>,
    frames: Condvar,
}

impl Shared {
    pub fn update(&self, update: impl FnOnce(&mut BrowserState)) {
        let mut state = self.state.lock();
        update(&mut state);
        state.revision += 1;
    }

    pub fn signal_frame(&self) {
        *self.frame_generation.lock() += 1;
        self.frames.notify_all();
    }
}

pub struct MolioBrowser {
    shared: Arc<Shared>,
    commands: mpsc::UnboundedSender<Command>,
    worker: Option<JoinHandle<()>>,
}

type StateCallback = unsafe extern "C" fn(*mut c_void, *const u8, usize);
type FrameCallback = unsafe extern "C" fn(*mut c_void, *const u8, u32, u32) -> bool;

/// # Safety
/// The profile must be an accessible NUL-terminated UTF-8 path for the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn molio_browser_open(profile: *const c_char) -> *mut MolioBrowser {
    if profile.is_null() {
        return std::ptr::null_mut();
    }
    // SAFETY: The caller owns the profile string for the duration of the call.
    let Ok(profile) = (unsafe { CStr::from_ptr(profile) }).to_str() else {
        return std::ptr::null_mut();
    };
    let profile = PathBuf::from(profile);
    if !profile.is_absolute() {
        return std::ptr::null_mut();
    }
    let shared = Arc::new(Shared::default());
    let (commands, receiver) = mpsc::unbounded_channel();
    let worker = {
        let shared = shared.clone();
        thread::Builder::new()
            .name("molio-browser".into())
            .spawn(move || {
                if let Err(error) = browser::run(profile, receiver, shared.clone()) {
                    shared.update(|state| state.error = Some(format!("{error:#}")));
                }
            })
    };
    match worker {
        Ok(worker) => Box::into_raw(Box::new(MolioBrowser {
            shared,
            commands,
            worker: Some(worker),
        })),
        Err(_) => std::ptr::null_mut(),
    }
}

/// # Safety
/// The handle must be valid and exclusively owned. The presenter must have
/// been detached, and the handle must not be used after the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn molio_browser_close(browser: *mut MolioBrowser) {
    if browser.is_null() {
        return;
    }
    // SAFETY: The caller transfers ownership of the sole handle.
    let mut browser = unsafe { Box::from_raw(browser) };
    let _ = browser.commands.send(Command::Shutdown);
    if let Some(worker) = browser.worker.take() {
        let _ = worker.join();
    }
}

/// # Safety
/// The handle and the NUL-terminated JSON command must be valid during the call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn molio_browser_send(
    browser: *const MolioBrowser,
    json: *const c_char,
) -> bool {
    if browser.is_null() || json.is_null() {
        return false;
    }
    // SAFETY: Both pointers are borrowed for this call by the Node-API bridge.
    let (browser, bytes) = unsafe { (&*browser, CStr::from_ptr(json).to_bytes()) };
    let Ok(command) = serde_json::from_slice(bytes) else {
        return false;
    };
    browser.commands.send(command).is_ok()
}

/// # Safety
/// The handle and callback must be valid for the call. The callback may only
/// borrow the JSON bytes and must not reenter the browser.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn molio_browser_state(
    browser: *const MolioBrowser,
    after: u64,
    callback: StateCallback,
    context: *mut c_void,
) {
    // SAFETY: The Node-API bridge keeps the handle alive.
    let Some(browser) = (unsafe { browser.as_ref() }) else {
        return;
    };
    let state = browser.shared.state.lock();
    if state.revision != after
        && let Ok(bytes) = serde_json::to_vec(&*state)
    {
        // SAFETY: JSON bytes are valid until the synchronous callback returns.
        unsafe { callback(context, bytes.as_ptr(), bytes.len()) };
    }
}

/// # Safety
/// The caller must keep the browser alive while the presenter waits.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn molio_browser_wait_frame(browser: *const MolioBrowser, after: u64) -> u64 {
    // SAFETY: The presenter joins before the Node-API bridge closes the handle.
    let Some(browser) = (unsafe { browser.as_ref() }) else {
        return after;
    };
    let shared = &browser.shared;
    let mut generation = shared.frame_generation.lock();
    let deadline = Instant::now() + Duration::from_millis(40);
    while *generation == after
        && !shared
            .frames
            .wait_until(&mut generation, deadline)
            .timed_out()
    {}
    *generation
}

/// # Safety
/// The handle and callback must be valid for the call. The callback may only
/// borrow the tightly packed BGRA pixels and must not reenter the browser.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn molio_browser_frame(
    browser: *const MolioBrowser,
    callback: FrameCallback,
    context: *mut c_void,
) -> bool {
    // SAFETY: The presenter joins before the Node-API bridge closes the handle.
    let Some(browser) = (unsafe { browser.as_ref() }) else {
        return false;
    };
    let output = browser.shared.viewport.lock().clone();
    output
        .and_then(|output| {
            output.read(|frame| {
                frame.pixels.is_some_and(|pixels| {
                    // SAFETY: The viewport keeps this frame pinned for the entire callback.
                    unsafe {
                        callback(
                            context,
                            pixels.as_ptr(),
                            frame.layout.width,
                            frame.layout.height,
                        )
                    }
                })
            })
        })
        .unwrap_or(false)
}
