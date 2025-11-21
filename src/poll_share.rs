use core::{cell::RefCell, mem::MaybeUninit};
use rtic::Mutex;

pub trait NetMutex: Mutex<T = crate::Net> + 'static {}
impl<T: Mutex<T = crate::Net> + 'static> NetMutex for T {}

#[derive(Debug)]
pub struct TokenProvider<R: 'static>(&'static RefCell<R>);
pub struct TokenProviderPlace<R: Mutex>(MaybeUninit<RefCell<R>>);

impl<R> Copy for TokenProvider<R> {}
impl<R> Clone for TokenProvider<R> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<R: Mutex> TokenProviderPlace<R> {
    pub const fn new() -> Self {
        Self(MaybeUninit::uninit())
    }
}

impl<R: Mutex + 'static> TokenProvider<R> {
    pub fn new(place: &'static mut TokenProviderPlace<R>, value: R) -> Self {
        Self(place.0.write(RefCell::new(value)))
    }
    /// Lock the resource and provide it to the closure.
    ///
    /// # Panics
    ///
    /// When entered recursively.
    ///
    pub fn lock<Ret>(&self, f: impl FnOnce(&mut R::T) -> Ret) -> Ret {
        self.0.borrow_mut().lock(f)
    }
}
