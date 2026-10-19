use std::sync::Arc;
use std::time::Instant;

use core_lib::circle::Circle;
use core_lib::ext::Ext;
use core_lib::model::{Inner, make_engine};
use core_lib::observer::Observer;
use core_lib::square::Square;
use core_lib::Engine;

pub struct Runner {
    pub engine: Engine,
    pub boxed: Box<Engine>,
    pub shared: Arc<Engine>,
}

impl Runner {
    fn go(&self) {}

    pub fn run(&mut self) {
        // Same file: no edge.
        self.go();
        // Field, field chain, smart pointers, lock guards, RefCell.
        self.engine.start();
        self.engine.inner.poke();
        self.boxed.start();
        self.shared.start();
        self.engine.shared.lock().unwrap().bump();
        self.engine.cell.borrow_mut().bump();
        self.engine.clock.tick();
    }
}

pub fn calls(e: &mut Engine, o: &dyn Observer, sq: Square, c: Circle) -> std::io::Result<()> {
    e.start();
    let x: Engine = Engine::new();
    x.start();
    let y = Engine::new();
    y.start();
    let z = Engine::open("path")?;
    z.start();
    let w = make_engine();
    w.inner().poke();
    o.notify();
    // Two workspace types with the same method name.
    sq.area();
    c.area();
    // External receivers whose method names exist in `decoy.rs`.
    let t = Instant::now();
    t.elapsed();
    let mut rx: tokio::sync::mpsc::Receiver<u8> = make_rx();
    rx.recv_many();
    // A derived method on a workspace type: `Decoy::eq` must not be linked.
    x.eq(&y);
    // An extension trait on an external type.
    let v: Vec<u8> = Vec::new();
    v.ext();
    Ok(())
}

pub fn generic<G>(g: G)
where
    G: Observer,
{
    g.notify();
}

pub fn opaque(o: impl Observer) {
    o.notify();
}

pub fn shadow(e: Engine) {
    let e: Square = Square;
    e.area();
}

pub fn inner_of(i: &Inner) {
    i.poke();
}

fn make_rx() -> tokio::sync::mpsc::Receiver<u8> {
    todo!()
}
