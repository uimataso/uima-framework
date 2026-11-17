use uima_port_macro::{HasPort, has_port};

#[has_port]
pub trait Clock {}

struct System;
impl Clock for System {}

#[derive(HasPort)]
struct Ctx {
    #[has_port(Clock)]
    a: System,
    #[has_port(Clock)]
    b: System,
}

fn main() {}
