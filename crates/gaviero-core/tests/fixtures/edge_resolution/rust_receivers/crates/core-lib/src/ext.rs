pub trait Ext {
    fn ext(&self);
}

impl Ext for Vec<u8> {
    fn ext(&self) {}
}
