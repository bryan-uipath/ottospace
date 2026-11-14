//! The clipboard. Every copy and paste of ottospace goes through here, so one place sees them
//! all. A test run (`OTTOSPACE_E2E`) has a stand-in in memory: tests never touch the user's.

use std::path::PathBuf;

use gpui::{App, ClipboardItem};

/// Where copied text goes, and pasted text comes from.
trait Board {
    fn write(&mut self, text: String, cx: &mut App);
    /// `None`: it holds no text, e.g. only an image.
    fn read(&self, cx: &App) -> Option<String>;
    /// Files copied in Finder.
    fn files(&self) -> Vec<PathBuf>;
    /// The stand-in's text, for a test's state. The system's is never shown.
    fn test_text(&self) -> Option<&str>;
}

/// The general pasteboard.
struct System;

impl Board for System {
    fn write(&mut self, text: String, cx: &mut App) {
        cx.write_to_clipboard(ClipboardItem::new_string(text));
    }

    fn read(&self, cx: &App) -> Option<String> {
        cx.read_from_clipboard().and_then(|item| item.text())
    }

    fn files(&self) -> Vec<PathBuf> {
        ottospace_mac::pasteboard::files()
    }

    fn test_text(&self) -> Option<&str> {
        None
    }
}

/// A test run's: the last text copied.
#[derive(Default)]
struct Memory(Option<String>);

impl Board for Memory {
    fn write(&mut self, text: String, _: &mut App) {
        self.0 = Some(text);
    }

    fn read(&self, _: &App) -> Option<String> {
        self.0.clone()
    }

    fn files(&self) -> Vec<PathBuf> {
        Vec::new()
    }

    fn test_text(&self) -> Option<&str> {
        self.0.as_deref()
    }
}

pub struct Clipboard(Box<dyn Board>);

impl Clipboard {
    /// The system's, or with `test` a stand-in.
    pub fn new(test: bool) -> Self {
        Self(if test { Box::new(Memory::default()) } else { Box::new(System) })
    }

    pub fn copy(&mut self, text: String, cx: &mut App) {
        self.0.write(text, cx);
    }

    /// The text to paste. `None`: no text, or empty.
    pub fn text(&self, cx: &App) -> Option<String> {
        self.0.read(cx).filter(|text| !text.is_empty())
    }

    pub fn files(&self) -> Vec<PathBuf> {
        self.0.files()
    }

    /// What a test run's stand-in holds; `None` outside one.
    pub fn test_text(&self) -> Option<&str> {
        self.0.test_text()
    }
}
