extern crate beta_core;

mod engine;

use beta_core::shapes::widget::Widget;
use beta_core::util::{self, helper as h};
use beta_core::Gadget;
use engine::*;

fn main() {
    let _a = h();
    let _b = util::only_here();
    let w = Widget::new();
    let _g = Gadget::new();
    run_engine();
    w.spin();
    let _c = w.describe();
    let _m = std::cmp::max(1, 2);
    local();
    let _d = beta_core::util::helper();
}

fn local() {}
