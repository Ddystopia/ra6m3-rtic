use core::{cell::Cell, ops::Deref};

use crate::log::*;
use cortex_m::interrupt::Mutex;

pub fn exit() -> ! {
    use cortex_m::asm::bkpt;

    loop {
        bkpt();
    }
}

#[derive(Debug, PartialEq, Eq)]
#[cfg_attr(feature = "defmt", derive(defmt::Format))]
pub struct ExtendRefGuard<T: ?Sized> {
    id: u128,
    value: *const T,
}

pub fn extend_ref<T: ?Sized, R>(
    value: &T,
    f: impl FnOnce(ExtendRefGuard<T>) -> (ExtendRefGuard<T>, R),
) -> R {
    static ID: Mutex<Cell<u128>> = Mutex::new(Cell::new(0));

    let Some(id) = cortex_m::interrupt::free(|cs| {
        let borrow = ID.borrow(cs);
        let id = borrow.get();
        if let Some(next) = id.checked_add(1) {
            borrow.set(next);
            Some(id)
        } else {
            None
        }
    }) else {
        error!("ExtendRefGuard::extend_ref: somehow 2**128 ids are exhausted");
        exit();
    };

    let (guard, ret) = f(ExtendRefGuard { id, value });

    if guard.id != id || !core::ptr::eq(guard.value, value) {
        error!("ExtendRefGuard::extend_ref: id mismatch");

        exit();
    }

    ret
}

impl<T: ?Sized> Deref for ExtendRefGuard<T> {
    type Target = T;
    fn deref(&self) -> &Self::Target {
        // SAFETY: `self.value` is guaranteed be be alive because `extend_ref` is
        // blocking and keeping `value` alive until it gets `ExtendRefGuard<T>`
        // and asserts its id. And this crate is using `panic = abort`, so no unwinding.
        unsafe { &*(self.value as *const T) }
    }
}
