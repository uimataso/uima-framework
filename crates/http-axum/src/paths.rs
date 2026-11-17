//! [`Paths`], the request paths a layer is configured with.

use std::sync::Arc;

/// Request paths, each matching only a request path that is the same.
#[derive(Debug, Clone, Default)]
pub(crate) struct Paths(Arc<[String]>);

impl Paths {
    /// These paths and `paths`.
    ///
    /// # Panics
    ///
    /// If one of `paths` doesn't start with `/`, since it could never match.
    pub(crate) fn with<P: AsRef<str>>(&self, paths: impl IntoIterator<Item = P>) -> Self {
        let added = paths.into_iter().map(|path| {
            let path = path.as_ref();
            assert!(
                path.starts_with('/'),
                "the path {path:?} doesn't start with `/`"
            );
            path.to_owned()
        });
        Self(self.0.iter().cloned().chain(added).collect())
    }

    pub(crate) fn contains(&self, path: &str) -> bool {
        self.0.iter().any(|own| own == path)
    }
}

#[cfg(test)]
mod tests {
    use uima_testing::check;

    use super::*;

    #[test]
    fn paths_match_exactly() {
        const PATHS: &[&str] = &["/hook"];
        let paths = Paths::default().with(PATHS).with(vec![String::from("/b")]);
        check!(paths.contains("/hook"));
        check!(paths.contains("/b"));
        for other in ["/hook/", "/HOOK", "//hook", "/ho%6Fk", "/hooks", "/", ""] {
            check!(!paths.contains(other), "{other}");
        }
    }

    #[test]
    #[should_panic(expected = "the path \"hook\" doesn't start with `/`")]
    fn a_path_without_a_leading_slash_panics() {
        Paths::default().with(["hook"]);
    }
}
