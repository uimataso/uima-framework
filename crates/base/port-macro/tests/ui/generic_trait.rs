#[uima_port_macro::has_port]
pub trait Repo<T> {
    fn get(&self) -> T;
}

fn main() {}
