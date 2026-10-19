use beta_core::observe::Observer;

pub struct Part;

impl Part {
    pub fn new() -> Self {
        Part
    }

    pub fn tick(&self) {}
}

impl Observer for Part {
    fn notify(&self) {}
}

pub fn assemble() {}

pub fn alert(o: &dyn Observer) {
    o.notify();
}
