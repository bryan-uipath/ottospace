//! Apple's on-device recogniser (`SpeechAnalyzer` with a `SpeechTranscriber`, macOS 26) through
//! the Swift shim in `swift/listen`, built by `build.rs`. Nothing leaves the Mac: the transcriber
//! sends no audio anywhere, and its model is the system's, installed by the system. Below macOS
//! 26, or built with an older SDK, every call says it is not here.

use std::ffi::{CStr, CString, c_char, c_void};

/// What a result callback is: a line, an error, or the end.
type Callback = extern "C" fn(*mut c_void, i32, *const c_char, u64, u64, bool);
type Done = extern "C" fn(*mut c_void, bool, *const c_char);

unsafe extern "C" {
    fn ottospace_listen_status(locale: *const c_char) -> i32;
    fn ottospace_listen_install(locale: *const c_char, done: Done, context: *mut c_void);
    fn ottospace_listen_new(locale: *const c_char, rate: f64, callback: Callback, context: *mut c_void) -> *mut c_void;
    fn ottospace_listen_feed(handle: *mut c_void, samples: *const f32, n: isize);
    fn ottospace_listen_finish(handle: *mut c_void);
}

/// Whether the recogniser can hear `locale` here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    /// Below macOS 26, or built without its SDK.
    Absent,
    /// The language has no model.
    Unsupported,
    /// Supported, its model not installed: the system can download it.
    NotInstalled,
    Downloading,
    Installed,
}

/// The locale as C, or null for the system's.
fn c_locale(locale: Option<&str>) -> Option<CString> {
    locale.and_then(|locale| CString::new(locale).ok())
}

fn ptr(locale: &Option<CString>) -> *const c_char {
    locale.as_ref().map_or(std::ptr::null(), |locale| locale.as_ptr())
}

/// Asks the system, and waits: call it off the main thread.
pub fn status(locale: Option<&str>) -> Status {
    let locale = c_locale(locale);
    match unsafe { ottospace_listen_status(ptr(&locale)) } {
        1 => Status::NotInstalled,
        2 => Status::Installed,
        3 => Status::Downloading,
        4 => Status::Unsupported,
        _ => Status::Absent,
    }
}

/// Asks the system to download and install the model; `done` on another thread, when it is in or why not.
pub fn install(locale: Option<&str>, done: impl FnOnce(Result<(), String>) + Send + 'static) {
    extern "C" fn finished(context: *mut c_void, ok: bool, why: *const c_char) {
        let done = unsafe { Box::from_raw(context.cast::<Box<dyn FnOnce(Result<(), String>) + Send>>()) };
        let why = (!why.is_null()).then(|| unsafe { CStr::from_ptr(why) }.to_string_lossy().into_owned());
        done(if ok { Ok(()) } else { Err(why.unwrap_or_default()) });
    }
    let done: Box<Box<dyn FnOnce(Result<(), String>) + Send>> = Box::new(Box::new(done));
    let locale = c_locale(locale);
    unsafe { ottospace_listen_install(ptr(&locale), finished, Box::into_raw(done).cast()) };
}

/// What the recogniser says.
#[derive(Clone, Debug, PartialEq)]
pub enum Heard {
    /// A line, volatile until `is_final`; times in ms of the audio fed.
    Line { text: String, start_ms: u64, end_ms: u64, is_final: bool },
    Failed(String),
    /// No more: after `Recogniser` is dropped, the last lines made final.
    Finished,
}

type Sink = Box<dyn Fn(Heard) + Send + Sync>;

/// One stream's recogniser. Dropped: its input ends, its last lines come, then `Finished`.
pub struct Recogniser {
    handle: *mut c_void,
}

// The shim locks around each feed.
unsafe impl Send for Recogniser {}
unsafe impl Sync for Recogniser {}

extern "C" fn heard(context: *mut c_void, kind: i32, text: *const c_char, start: u64, end: u64, is_final: bool) {
    let text = || (!text.is_null()).then(|| unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned());
    let sink = context.cast::<Sink>();
    let said = match kind {
        0 => Heard::Line { text: text().unwrap_or_default(), start_ms: start, end_ms: end, is_final },
        1 => Heard::Failed(text().unwrap_or_default()),
        _ => Heard::Finished,
    };
    let last = said == Heard::Finished;
    unsafe { (*sink)(said) };
    // The shim calls nothing after it: its context goes.
    if last {
        drop(unsafe { Box::from_raw(sink) });
    }
}

impl Recogniser {
    /// A recogniser of `rate` Hz mono samples in `locale` (or the system's). None: the model is not
    /// installed, or there is no recogniser here.
    pub fn new(locale: Option<&str>, rate: f64, on: impl Fn(Heard) + Send + Sync + 'static) -> Option<Self> {
        let sink: *mut Sink = Box::into_raw(Box::new(Box::new(on)));
        let locale = c_locale(locale);
        let handle = unsafe { ottospace_listen_new(ptr(&locale), rate, heard, sink.cast()) };
        if handle.is_null() {
            drop(unsafe { Box::from_raw(sink) });
            return None;
        }
        Some(Recogniser { handle })
    }

    pub fn feed(&self, samples: &[f32]) {
        unsafe { ottospace_listen_feed(self.handle, samples.as_ptr(), samples.len() as isize) };
    }
}

impl Drop for Recogniser {
    fn drop(&mut self) {
        unsafe { ottospace_listen_finish(self.handle) };
    }
}
