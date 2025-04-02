#![allow(dead_code)]

use core::{future::poll_fn, pin::pin};

use heapless::mpmc::MpMcQueue;
use rtic_common::wait_queue::{Link, WaitQueue};

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
        self.queue.enqueue(value)?;

        if let Some(waker) = self.rx_wait_queue.pop() {
            waker.wake();
        }

        Ok(())
    }

    pub fn try_recv(&self) -> Option<T> {
        let val = self.queue.dequeue()?;
        if let Some(waker) = self.tx_wait_queue.pop() {
            waker.wake();
        }
        Some(val)
    }

    pub async fn send(&self, value: T) {
        let mut value = Some(value);

        let link = pin!(None::<Link<_>>);
        let mut link = OnDrop::new(link, |link| {
            if let Some(link) = link.as_mut().as_pin_mut() {
                link.as_ref()
                    .get_ref()
                    .remove_from_list(&self.tx_wait_queue);
            }
        });

        poll_fn(|cx| match self.try_send(value.take().unwrap()) {
            Ok(()) => {
                link.0.set(None);
                core::task::Poll::Ready(())
            }
            Err(old_val) => {
                let new_link = Link::new(cx.waker().clone());
                link.0.set(Some(new_link));

                if let Some(link) = link.0.as_ref().as_pin_ref() {
                    // SAFETY: codebase assumes that drop handler is always called
                    unsafe {
                        self.tx_wait_queue.push(link);
                    }
                }
                value = Some(old_val);
                core::task::Poll::Pending
            }
        })
        .await
    }

    pub async fn recv(&self) -> T {
        let link = pin!(None::<Link<_>>);
        let mut link = OnDrop::new(link, |link| {
            if let Some(link) = link.as_mut().as_pin_mut() {
                link.as_ref()
                    .get_ref()
                    .remove_from_list(&self.rx_wait_queue);
            }
        });

        poll_fn(|cx| match self.try_recv() {
            Some(value) => {
                if let Some(link) = link.0.as_mut().as_pin_mut() {
                    link.as_ref()
                        .get_ref()
                        .remove_from_list(&self.rx_wait_queue);
                }
                link.0.set(None);
                core::task::Poll::Ready(value)
            }
            None => {
                let new_link = Link::new(cx.waker().clone());
                link.0.set(Some(new_link));

                if let Some(link) = link.0.as_ref().as_pin_ref() {
                    link.remove_from_list(&self.tx_wait_queue);

                    // SAFETY: Clean up will be performed eigher in the
                    // - Drop handler of the future
                    // - Before new link replaced
                    // - When ready
                    unsafe {
                        self.rx_wait_queue.push(link);
                    }
                }
                core::task::Poll::Pending
            }
        })
        .await
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
