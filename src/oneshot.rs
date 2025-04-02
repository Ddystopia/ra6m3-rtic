#![allow(dead_code)]

use atomic_refcell::AtomicRefCell;
use diatomic_waker::{DiatomicWaker, WakeSinkRef, WakeSourceRef};

// INVARIANT: `place_1` and `place_2` cannot be `borrow` or `borrow_mut` at the same time, one of them shall be unlocked.

pub struct OneShot<T> {
    place_1: AtomicRefCell<Option<T>>,
    place_2: AtomicRefCell<Option<T>>,
    waker: DiatomicWaker,
}

pub struct Receiver<'a, T> {
    sink: WakeSinkRef<'a>,
    place_1: &'a AtomicRefCell<Option<T>>,
    place_2: &'a AtomicRefCell<Option<T>>,
}

pub struct Sender<'a, T> {
    source: WakeSourceRef<'a>,
    place_1: &'a AtomicRefCell<Option<T>>,
    place_2: &'a AtomicRefCell<Option<T>>,
}

impl<T> OneShot<T> {
    pub const fn new() -> Self {
        Self {
            place_1: AtomicRefCell::new(None),
            place_2: AtomicRefCell::new(None),
            waker: DiatomicWaker::new(),
        }
    }
    pub fn split<'a>(&'a mut self) -> (Sender<'a, T>, Receiver<'a, T>) {
        let sink = self.waker.sink_ref();
        let source = sink.source_ref();
        (
            Sender {
                source,
                place_1: &self.place_1,
                place_2: &self.place_2,
            },
            Receiver {
                sink,
                place_1: &self.place_1,
                place_2: &self.place_2,
            },
        )
    }
}

impl<'a, T> Sender<'a, T> {
    pub fn send(self, value: T) {
        if let Ok(mut place_1) = self.place_1.try_borrow_mut() {
            *place_1 = Some(value);
            self.source.notify();
            return;
        }

        if let Ok(mut place_2) = self.place_2.try_borrow_mut() {
            *place_2 = Some(value);
            self.source.notify();
            return;
        }

        panic!("OneShot: both places are already borrowed");
    }
}

impl<'a, T> Receiver<'a, T> {
    pub async fn receive(mut self) -> T {
        let sink = &mut self.sink;

        sink.wait_until(|| {
            if let Ok(mut place_1) = self.place_1.try_borrow_mut() {
                if let Some(value) = place_1.take() {
                    return Some(value);
                }
            }

            if let Ok(mut place_2) = self.place_2.try_borrow_mut() {
                if let Some(value) = place_2.take() {
                    return Some(value);
                }
            }

            None
        })
        .await
    }
}
