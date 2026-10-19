pub mod parts;

use self::parts::Part;

pub fn run_engine() {
    let p = Part::new();
    p.tick();
    crate::engine::parts::assemble();
}
