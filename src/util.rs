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
