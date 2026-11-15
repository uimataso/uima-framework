#[uima_port_macro::has_port]
pub trait Clock {
    fn clock(&self) -> u64;
}

fn main() {}
