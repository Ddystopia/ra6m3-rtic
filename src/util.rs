use core::mem::transmute;
use core::task::{RawWaker, RawWakerVTable, Waker};

pub const fn waker(f: fn()) -> Waker {
    static VTABLE: RawWakerVTable = unsafe {
        RawWakerVTable::new(
            |this| RawWaker::new(this, &VTABLE),
            |this| transmute::<*const (), fn()>(this)(),
            |this| transmute::<*const (), fn()>(this)(),
            |_| {},
        )
    };
    let raw = RawWaker::new(f as *const (), &VTABLE);

    unsafe { Waker::from_raw(raw) }
}

#[expect(dead_code)]
pub async fn get_waker() -> Waker {
    use core::{future::poll_fn, task::Poll};

    poll_fn(|cx| Poll::Ready(cx.waker().clone())).await
}
