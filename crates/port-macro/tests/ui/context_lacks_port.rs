use uima_port_macro::{HasPort, has_port};

#[has_port]
pub trait Clock {}

#[has_port]
pub trait Mailer {}

struct System;
impl Clock for System {}

#[derive(HasPort)]
struct Ctx {
    #[has_port(Clock)]
    clock: System,
}

fn send<C: HasMailer>(_: &C) {}

fn main() {
    send(&Ctx { clock: System });
}
