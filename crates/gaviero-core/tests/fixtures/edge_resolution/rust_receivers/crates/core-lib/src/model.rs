use std::cell::RefCell;
use std::sync::{Arc, Mutex};

use crate::clock::Clock;

pub struct State {
    pub count: u32,
}

impl State {
    pub fn bump(&mut self) {
        self.count += 1;
    }
}

pub struct Inner {
    pub state: State,
}

impl Inner {
    pub fn poke(&self) {}
}

#[derive(PartialEq)]
pub struct Engine {
    pub inner: Inner,
    pub shared: Arc<Mutex<State>>,
    pub cell: RefCell<State>,
    pub clock: Clock,
}

impl Engine {
    pub fn new() -> Self {
        todo!()
    }

    pub fn open(path: &str) -> std::io::Result<Engine> {
        todo!()
    }

    pub fn inner(&self) -> &Inner {
        &self.inner
    }

    pub fn start(&self) {}
}

pub fn make_engine() -> Engine {
    todo!()
}
