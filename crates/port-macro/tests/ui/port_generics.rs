use uima_port_macro::HasPort;

#[derive(HasPort)]
struct Ctx {
    #[has_port(Clock<u8>)]
    clock: u8,
}

fn main() {}
