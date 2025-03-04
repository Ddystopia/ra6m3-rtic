use atomic_refcell::AtomicRefCell;
use extend_mut::ExtendMut;
use smoltcp::iface::{Interface, SocketSet};

// It doesn't need to be `AtomicRefCell`, 2 `AtomicPtr` would be enough, but
// I don't want unsafe code to convert pointers to `&'static mut` later.
#[derive(Default)]
pub struct MqttNetChannel(AtomicRefCell<Option<Payload>>);
pub struct FedMqttToken(Private);

struct Private;

struct Payload {
    iface: &'static mut Interface,
    sockets: &'static mut SocketSet<'static>,
}

impl MqttNetChannel {
    pub const fn new() -> Self {
        Self(AtomicRefCell::new(None))
    }

    pub fn put(&self, iface: &'static mut Interface, sockets: &'static mut SocketSet<'static>) {
        self.0.borrow_mut().replace(Payload { iface, sockets });
    }
    pub fn take(&self) -> (&'static mut Interface, &'static mut SocketSet<'static>) {
        let Payload { iface, sockets } = self.0.borrow_mut().take().unwrap();

        (iface, sockets)
    }
    pub fn feed<R>(
        &self,
        iface: &mut Interface,
        sockets: &mut SocketSet<'static>,
        f: impl FnOnce(FedMqttToken) -> R,
    ) -> R {
        (sockets, iface).extend_mut(|(sockets, iface)| {
            self.put(iface, sockets);
            let ret = f(FedMqttToken(Private));
            let (iface, sockets) = self.take();
            ((sockets, iface), ret)
        })
    }
}
