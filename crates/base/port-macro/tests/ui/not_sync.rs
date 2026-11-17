use std::cell::Cell;

use uima_port_macro::{HasPort, has_port};

#[has_port]
pub trait Clock {}

struct Counter(Cell<u64>);
impl Clock for Counter {}

#[derive(HasPort)]
struct Ctx {
    #[has_port(Clock)]
    clock: Counter,
}

fn main() {}
