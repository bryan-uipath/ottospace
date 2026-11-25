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
    fn ottospace_listen_new(
        locale: *const c_char,
        rate: f64,
        callback: Callback,
        context: *mut c_void,
        why: *mut i32,
    ) -> *mut c_void;
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
    /// macOS 26, but this Mac cannot run the transcriber (a virtual machine, for one).
    Unavailable,
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
        5 => Status::Unavailable,
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

/// Why a recogniser did not start: the shim's step that failed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NotStarted {
    /// Below macOS 26, or built without its SDK.
    Absent,
    /// No language of the system's has a model.
    NoLanguage,
    /// The model is supported but not installed, being downloaded, or not supported.
    NotInstalled,
    Downloading,
    ModelUnsupported,
    /// The transcriber takes no audio format.
    NoFormat,
    /// The sample rate given is no audio format.
    BadRate,
    /// A code this side does not know.
    Other(i32),
}

impl NotStarted {
    fn from_code(code: i32) -> Self {
        match code {
            1 => Self::Absent,
            2 => Self::NoLanguage,
            3 => Self::NotInstalled,
            4 => Self::Downloading,
            5 => Self::ModelUnsupported,
            6 => Self::NoFormat,
            7 => Self::BadRate,
            other => Self::Other(other),
        }
    }

    /// For the event log: `no_format`.
    pub fn code(self) -> String {
        match self {
            Self::Absent => "absent".into(),
            Self::NoLanguage => "no_language".into(),
            Self::NotInstalled => "not_installed".into(),
            Self::Downloading => "downloading".into(),
            Self::ModelUnsupported => "model_unsupported".into(),
            Self::NoFormat => "no_format".into(),
            Self::BadRate => "bad_rate".into(),
            Self::Other(code) => format!("code_{code}"),
        }
    }
}

impl std::fmt::Display for NotStarted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Absent => write!(f, "this build has no speech recogniser"),
            Self::NoLanguage => write!(f, "no language of the system's has a speech model"),
            Self::NotInstalled => write!(f, "the speech model is not installed"),
            Self::Downloading => write!(f, "the speech model is still downloading"),
            Self::ModelUnsupported => write!(f, "the speech model is not supported"),
            Self::NoFormat => write!(f, "the speech model takes no audio format"),
            Self::BadRate => write!(f, "the sound's rate is no audio format"),
            Self::Other(code) => write!(f, "step {code}"),
        }
    }
}

impl Recogniser {
    /// A recogniser of `rate` Hz mono samples in `locale` (or the system's), or the step that
    /// failed (`NotStarted`).
    pub fn new(
        locale: Option<&str>,
        rate: f64,
        on: impl Fn(Heard) + Send + Sync + 'static,
    ) -> Result<Self, NotStarted> {
        let sink: *mut Sink = Box::into_raw(Box::new(Box::new(on)));
        let locale = c_locale(locale);
        let mut why = 0;
        let handle = unsafe { ottospace_listen_new(ptr(&locale), rate, heard, sink.cast(), &mut why) };
        if handle.is_null() {
            drop(unsafe { Box::from_raw(sink) });
            return Err(NotStarted::from_code(why));
        }
        Ok(Recogniser { handle })
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
