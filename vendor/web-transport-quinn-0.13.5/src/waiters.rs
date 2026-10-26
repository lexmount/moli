//! The caller's end of a registration in a [`kio::Fan`].
//!
//! [`SessionAccept`](crate::SessionAccept) is shared by every clone of a session, so it
//! fans a single arrival out to every accepter parked on it. That list, its waker, and the
//! hold that keeps an inline wake from landing under the accept lock are all
//! [`kio::Fan`]. What is left here is the piece kio deliberately does not do.

use std::task::{Context, Poll};

use kio::{Park, Waiter};

/// A [`kio::Park`] that lets go of its waiter once the poll finishes.
///
/// `Park` retains the waiter until the next poll or until it drops, which is what a
/// pending operation needs: a registration in [`AcceptWaiters`] (or any
/// [`kio::WaiterList`]) lives only as long as the [`Waiter`] that made it. A *finished*
/// poll is the other case. The stream is done with that caller, and going on holding its
/// waker pins the polling task's allocation until something polls the cell again — for a
/// stream whose last read completed and then sat idle, that is the rest of the
/// connection. So this releases it on `Ready`.
///
/// `Park` also clones empty, so a cloned handle never inherits another handle's live
/// registration.
///
/// The `async` methods have no need for any of it — [`kio::wait`] keeps the waiter inside
/// the future it builds, so dropping the future drops the registration.
#[derive(Clone, Default)]
pub(crate) struct Parked(Park);

impl Parked {
    /// Run one poll with a retained waiter, releasing it if the poll finishes.
    pub(crate) fn poll<T>(
        &mut self,
        cx: &Context<'_>,
        poll: impl FnOnce(&Waiter) -> Poll<T>,
    ) -> Poll<T> {
        let waiter = self.hold(cx);
        let result = poll(&waiter);
        self.settle(&result);
        result
    }

    /// Hold a waiter for this poll, for a body that needs `&mut self` of the struct
    /// holding this cell — the borrow checker allows only one of those at a time, so the
    /// closure form above will not compile there. Pair it with [`settle`](Self::settle).
    ///
    /// The returned clone shares the parked waiter's identity, so nothing is lost by
    /// taking one, and it ends the borrow on the cell.
    pub(crate) fn hold(&mut self, cx: &Context<'_>) -> Waiter {
        self.0.hold(cx).clone()
    }

    /// Release the held waiter if the poll finished.
    pub(crate) fn settle<T>(&mut self, result: &Poll<T>) {
        if result.is_ready() {
            self.0 = Park::default();
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{atomic::AtomicBool, Arc};
    use std::task::Wake;

    use kio::Fan;

    use super::*;

    /// Records whether it was woken.
    #[derive(Default)]
    struct Flag {
        woken: AtomicBool,
    }

    impl Flag {
        fn woken(&self) -> bool {
            self.woken.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl Wake for Flag {
        fn wake(self: Arc<Self>) {
            self.wake_by_ref();
        }

        fn wake_by_ref(self: &Arc<Self>) {
            self.woken.store(true, std::sync::atomic::Ordering::SeqCst);
        }
    }

    /// A clone starts empty, so it must not carry the original's parked poller. The rest
    /// of the fan's behaviour is kio's and tested there; this is about `Parked`.
    #[test]
    fn cloning_a_pending_handle_does_not_retain_the_poller() {
        let fan = Fan::new();
        let flag = Arc::new(Flag::default());
        let waker = std::task::Waker::from(flag.clone());
        let mut parked = Parked::default();

        assert!(parked
            .poll(&Context::from_waker(&waker), |waiter| {
                fan.register(waiter);
                Poll::<()>::Pending
            })
            .is_pending());

        let cloned = parked.clone();
        drop(parked);
        fan.wake();

        assert!(
            !flag.woken(),
            "the clone retained the original handle's parked poller"
        );
        drop(cloned);
    }
}
