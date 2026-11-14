// The build reads a file outside the Rust sources, so it fails unless
// `extraSrc` lists it.
const NAME: &str = include_str!("../asset.txt");

fn main() {
    let _ = NAME;
}
