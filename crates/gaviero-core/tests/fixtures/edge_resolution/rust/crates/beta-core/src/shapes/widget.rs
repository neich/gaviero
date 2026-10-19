pub struct Widget;

impl Widget {
    pub fn new() -> Self {
        Widget
    }

    pub fn spin(&self) {}

    pub fn describe(&self) -> &'static str {
        "widget"
    }
}
