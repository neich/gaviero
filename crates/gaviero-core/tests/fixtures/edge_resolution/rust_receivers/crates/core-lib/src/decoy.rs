//! Methods named like std / tokio methods. Nothing in `app` uses `Decoy`,
//! so any edge from `app` to this file is a false positive.

pub struct Decoy;

impl Decoy {
    pub fn recv_many(&self) {}
    pub fn eq(&self, _other: &Decoy) -> bool {
        true
    }
    pub fn elapsed(&self) {}
}
