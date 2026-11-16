//! [`Secret`], a value that must stay out of logs.

use std::fmt;

/// A value that must stay out of logs and error messages, such as a password or a URL holding one.
///
/// ```
/// use uima_core::Secret;
///
/// let token = Secret::new(String::from("s3cr3t"));
/// assert_eq!(token.expose(), "s3cr3t");
/// assert_eq!(format!("{token:?}"), "Secret(***)");
/// ```
///
/// Reading it takes `expose` or `into_inner`, so the code that reads it shows that it does. It
/// has no `Display`, `Deref`, comparison or serde, and `Debug` prints `Secret(***)`. For a string
/// with rules, such as an API key, define a type with [`secret!`](crate::secret) instead.
#[derive(Clone)]
pub struct Secret<T>(T);

impl<T> Secret<T> {
    pub fn new(value: T) -> Self {
        Self(value)
    }

    /// The value, for the code that must use it.
    pub fn expose(&self) -> &T {
        &self.0
    }

    /// The value, taken out.
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> fmt::Debug for Secret<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
    }
}

#[cfg(test)]
mod tests {
    use uima_testing::check;

    use super::*;

    #[test]
    fn exposes_and_gives_back_the_value() {
        let secret = Secret::new(vec![1, 2, 3]);

        check!(secret.expose() == &vec![1, 2, 3]);
        check!(secret.into_inner() == vec![1, 2, 3]);
    }

    #[test]
    fn debug_hides_the_value() {
        check!(format!("{:?}", Secret::new("s3cr3t")) == "Secret(***)");
        check!(format!("{:#?}", Secret::new("s3cr3t")) == "Secret(***)");
        check!(format!("{:?}", Some(Secret::new(42))) == "Some(Secret(***))");
    }

    #[test]
    fn a_clone_holds_the_same_value() {
        let secret = Secret::new(String::from("s3cr3t"));

        check!(secret.clone().expose() == secret.expose());
    }
}
