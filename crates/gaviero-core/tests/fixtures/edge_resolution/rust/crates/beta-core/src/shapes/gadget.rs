use super::widget::Widget;
use crate::observe::Observer;

pub struct Gadget {
    w: Widget,
}

impl Gadget {
    pub fn new() -> Self {
        Gadget { w: Widget::new() }
    }

    pub fn describe(&self) -> &'static str {
        "gadget"
    }
}

impl Observer for Gadget {
    fn notify(&self) {}
}
