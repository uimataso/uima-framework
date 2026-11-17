use uima_port_macro::{HasPort, has_port};

#[has_port]
pub trait Clock {}

struct NotAClock;

#[derive(HasPort)]
struct Ctx {
    #[has_port(Clock)]
    clock: NotAClock,
}

fn main() {}
