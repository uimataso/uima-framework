fn main() {}

#[cfg(test)]
mod tests {
    // Clippy's `len_zero`, in code only `--all-targets` checks.
    fn blank(s: &str) -> bool {
        s.len() == 0
    }

    #[test]
    fn empty() {
        assert!(blank(""));
    }
}
