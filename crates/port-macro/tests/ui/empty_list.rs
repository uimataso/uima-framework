use uima_port_macro::HasPort;

#[derive(HasPort)]
struct Ctx {
    #[has_port()]
    clock: u8,
}

fn main() {}
