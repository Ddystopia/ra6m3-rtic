#![expect(dead_code)]
pub fn waker(f: fn()) -> core::task::Waker {
    use core::mem::transmute;
    use core::task::{RawWaker, RawWakerVTable, Waker};

    static VTABLE: RawWakerVTable = unsafe {
        RawWakerVTable::new(
            |this| RawWaker::new(this, &VTABLE),
            |this| transmute::<*const (), fn()>(this)(),
            |this| transmute::<*const (), fn()>(this)(),
            |_| {},
        )
    };
    unsafe {
        let raw = RawWaker::new(f as *const (), &VTABLE);
        Waker::from_raw(raw)
    }
}
