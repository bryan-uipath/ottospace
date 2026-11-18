//! The app's main loop sleeps while nothing happens. What happens, at its source (a hot key, a
//! zoom, a display change, a socket request, a terminal's output), posts a wakeup: `post`, from
//! any thread. The loop waits on `woken` and takes the wakeup with `take`. A routine result
//! (a file's unchanged stamp) only `nudge`s: the loop runs, but nothing is said to have happened.

use std::future::Future;
use std::pin::Pin;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::task::{Context, Poll, Waker};

static WOKEN: AtomicBool = AtomicBool::new(false);
static NUDGED: AtomicBool = AtomicBool::new(false);
static WAITER: Mutex<Option<Waker>> = Mutex::new(None);

/// Something happened: the loop runs soon. Cheap when posted again before the loop took it.
/// Not from a signal handler: it may lock.
pub fn post() {
    if !WOKEN.swap(true, SeqCst) {
        wake_waiter();
    }
}

/// A result to take: the loop runs soon, as at its slow step.
pub fn nudge() {
    if !NUDGED.swap(true, SeqCst) {
        wake_waiter();
    }
}

fn wake_waiter() {
    if let Some(waker) = WAITER.lock().ok().and_then(|mut waiter| waiter.take()) {
        waker.wake();
    }
}

/// True once after any `post` since the last take. Takes a nudge too.
pub fn take() -> bool {
    NUDGED.store(false, SeqCst);
    WOKEN.swap(false, SeqCst)
}

/// Ready once a wakeup is posted or nudged (not taken: `take` does that).
pub fn woken() -> Woken {
    Woken
}

pub struct Woken;

impl Future for Woken {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        // The waker first, then the flag: a post between the two is seen by the flag.
        if let Ok(mut waiter) = WAITER.lock() {
            *waiter = Some(cx.waker().clone());
        }
        if WOKEN.load(SeqCst) || NUDGED.load(SeqCst) { Poll::Ready(()) } else { Poll::Pending }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::task::Wake;

    struct Count(std::sync::atomic::AtomicUsize);
    impl Wake for Count {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, SeqCst);
        }
    }

    /// A post wakes the one waiting, once; posted again before the take, it wakes no one.
    #[test]
    fn a_post_wakes_the_waiting_loop_once() {
        take();
        let count = Arc::new(Count(Default::default()));
        let waker = Waker::from(count.clone());
        let mut cx = Context::from_waker(&waker);
        let mut woken = woken();
        assert!(Pin::new(&mut woken).poll(&mut cx).is_pending());
        post();
        post();
        assert_eq!(count.0.load(SeqCst), 1);
        assert!(Pin::new(&mut woken).poll(&mut cx).is_ready());
        assert!(take());
        assert!(!take());
        // A nudge wakes it, and is no wakeup to take.
        let mut woken = super::woken();
        assert!(Pin::new(&mut woken).poll(&mut cx).is_pending());
        nudge();
        assert_eq!(count.0.load(SeqCst), 2);
        assert!(Pin::new(&mut woken).poll(&mut cx).is_ready());
        assert!(!take());
        assert!(Pin::new(&mut woken).poll(&mut cx).is_pending());
    }
}
