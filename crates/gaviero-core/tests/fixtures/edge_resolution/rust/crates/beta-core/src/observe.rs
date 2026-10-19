pub trait Observer {
    fn notify(&self);
}

pub fn broadcast(observers: &[&dyn Observer]) {
    for o in observers {
        o.notify();
    }
}
