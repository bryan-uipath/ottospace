//! The menu bar's mark while a transcript is on: a red dot (an `NSStatusItem`), its menu Stop and
//! Show. Shown and hidden by the shell from the model; a choice in its menu is read by `take`.
//! Only the user's own mouse reaches a status item's menu: no client of the socket can.

use std::cell::RefCell;
use std::sync::atomic::{AtomicU8, Ordering};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObject};
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{NSMenu, NSMenuItem, NSStatusBar, NSStatusItem, NSVariableStatusItemLength};
use objc2_foundation::NSString;

/// What the user chose in the menu: nothing, Stop, Show.
static CHOSEN: AtomicU8 = AtomicU8::new(0);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Chosen {
    Stop,
    Show,
}

define_class!(
    // The menu items' target.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "OttospaceListenMenuTarget"]
    struct Target;

    impl Target {
        #[unsafe(method(stop:))]
        fn stop(&self, _sender: Option<&AnyObject>) {
            CHOSEN.store(1, Ordering::Relaxed);
        }

        #[unsafe(method(show:))]
        fn show(&self, _sender: Option<&AnyObject>) {
            CHOSEN.store(2, Ordering::Relaxed);
        }
    }
);

struct Shown {
    item: Retained<NSStatusItem>,
    /// The menu items hold their target weakly: it is kept here.
    _target: Retained<Target>,
}

thread_local! {
    static SHOWN: RefCell<Option<Shown>> = const { RefCell::new(None) };
}

/// Shows the dot (`title` beside it, e.g. `Transcribing`), or takes it away. On the main thread.
pub fn set(on: Option<&str>) {
    let Some(mtm) = MainThreadMarker::new() else { return };
    SHOWN.with_borrow_mut(|shown| match (on, shown.as_ref()) {
        (None, Some(_)) => {
            if let Some(gone) = shown.take() {
                NSStatusBar::systemStatusBar().removeStatusItem(&gone.item);
            }
        }
        (Some(title), None) => *shown = Some(make(mtm, title)),
        _ => {}
    });
}

fn make(mtm: MainThreadMarker, title: &str) -> Shown {
    let item = NSStatusBar::systemStatusBar().statusItemWithLength(NSVariableStatusItemLength);
    if let Some(button) = item.button(mtm) {
        button.setTitle(&NSString::from_str("● "));
        button.setToolTip(Some(&NSString::from_str(title)));
    }
    let target: Retained<Target> = unsafe { msg_send![Target::alloc(mtm), init] };
    let menu = NSMenu::new(mtm);
    for (name, action) in [("Stop Transcribing", sel!(stop:)), ("Show the Transcript", sel!(show:))] {
        let entry = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(mtm),
                &NSString::from_str(name),
                Some(action),
                &NSString::from_str(""),
            )
        };
        unsafe { entry.setTarget(Some(&target)) };
        menu.addItem(&entry);
    }
    item.setMenu(Some(&menu));
    Shown { item, _target: target }
}

/// What the user chose since the last look, if anything.
pub fn take() -> Option<Chosen> {
    match CHOSEN.swap(0, Ordering::Relaxed) {
        1 => Some(Chosen::Stop),
        2 => Some(Chosen::Show),
        _ => None,
    }
}
