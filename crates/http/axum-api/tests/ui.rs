//! The compile errors of `#[derive(IntoResponse)]`, with where they point.

#[test]
fn ui() {
    trybuild::TestCases::new().compile_fail("tests/ui/*.rs");
}
