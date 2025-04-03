#![allow(dead_code)]

use heapless::mpmc::MpMcQueue;
use rtic_common::wait_queue::WaitQueue;

// todo: maybe write it without `!Forget` assumption by requiring providing an allocation?

pub struct MpMcChannel<T, const N: usize> {
    tx_wait_queue: WaitQueue,
    rx_wait_queue: WaitQueue,
    queue: MpMcQueue<T, N>,
}

struct OnDrop<T, F: FnOnce(&mut T)>(T, Option<F>);

impl<T, const N: usize> Default for MpMcChannel<T, N> {
    fn default() -> Self {
        Self::new()
    }
}

impl<T, const N: usize> MpMcChannel<T, N> {
    pub const fn new() -> Self {
        Self {
            tx_wait_queue: WaitQueue::new(),
            rx_wait_queue: WaitQueue::new(),
            queue: MpMcQueue::new(),
        }
    }

    pub fn try_send(&self, value: T) -> Result<(), T> {
        while let Some(waker) = self.rx_wait_queue.pop() {
            waker.wake();
        }

        self.queue.enqueue(value)
    }

    pub fn try_recv(&self) -> Option<T> {
        while let Some(waker) = self.rx_wait_queue.pop() {
            waker.wake();
        }

        self.queue.dequeue()
    }

    pub async fn send(&self, value: T) {
        let mut value = Some(value);
        self.tx_wait_queue
            .wait_until(|| {
                self.try_send(value.take().expect("Future is polled after completion"))
                    .map_err(|old_val| value = Some(old_val))
                    .ok()
            })
            .await
    }

    pub async fn recv(&self) -> T {
        self.rx_wait_queue.wait_until(|| self.try_recv()).await
    }
}

impl<T, F: FnOnce(&mut T)> OnDrop<T, F> {
    fn new(value: T, f: F) -> Self {
        Self(value, Some(f))
    }
}
impl<T, F: FnOnce(&mut T)> Drop for OnDrop<T, F> {
    fn drop(&mut self) {
        if let Some(f) = self.1.take() {
            f(&mut self.0);
        }
    }
}
