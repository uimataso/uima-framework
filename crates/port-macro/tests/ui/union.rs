use uima_port_macro::HasPort;

#[derive(HasPort)]
union Ctx {
    a: u8,
}

fn main() {}
