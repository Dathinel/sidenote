use std::time::Instant;

pub(crate) fn now() -> Instant {
    #[cfg(test)]
    if let Some(now) = fixture::now() {
        return now;
    }
    Instant::now()
}

#[cfg(test)]
pub(crate) mod fixture {
    use super::Instant;
    use std::{cell::RefCell, time::Duration};

    #[derive(Clone, Copy, PartialEq, Eq)]
    pub(crate) enum Phase {
        Launch,
        Spawn,
        Reply,
        Pipes,
    }

    struct Clock {
        now: Instant,
        end: Instant,
        ready: Box<dyn Fn(Phase) -> bool>,
    }

    thread_local! {
        static CLOCK: RefCell<Option<Clock>> = const { RefCell::new(None) };
    }

    pub(crate) struct FrozenClock;
    impl FrozenClock {
        pub(crate) fn until(budget: Duration, ready: impl Fn(Phase) -> bool + 'static) -> Self {
            let now = Instant::now();
            let clock = Clock {
                now,
                end: now + budget,
                ready: Box::new(ready),
            };
            CLOCK.with(|slot| assert!(slot.replace(Some(clock)).is_none()));
            Self
        }
    }

    impl Drop for FrozenClock {
        fn drop(&mut self) {
            CLOCK.with(|slot| slot.take());
        }
    }

    pub(super) fn now() -> Option<Instant> {
        CLOCK.with(|slot| slot.borrow().as_ref().map(|clock| clock.now))
    }

    pub(crate) fn reach(phase: Phase) {
        CLOCK.with(|slot| {
            if let Some(clock) = slot.borrow_mut().as_mut()
                && (clock.ready)(phase)
            {
                clock.now = clock.end;
            }
        });
    }
}
