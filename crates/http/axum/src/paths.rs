//! [`Paths`], the request paths a layer is configured with.

use std::sync::Arc;

/// Request paths, each matching only a request path that is the same.
#[derive(Debug, Clone, Default)]
pub(crate) struct Paths(Arc<[String]>);

impl Paths {
    /// Adds `paths`.
    ///
    /// # Panics
    ///
    /// If one of `paths` doesn't start with `/` or holds a `?` or `#`, as no request's path does.
    pub(crate) fn extend<P: AsRef<str>>(&mut self, paths: impl IntoIterator<Item = P>) {
        let added = paths.into_iter().map(|path| {
            let path = path.as_ref();
            assert!(
                path.starts_with('/') && !path.contains(['?', '#']),
                "the path {path:?} can't match a request's path"
            );
            path.to_owned()
        });

        self.0 = self.0.iter().cloned().chain(added).collect();
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
        let mut paths = Paths::default();
        paths.extend(PATHS);
        paths.extend(vec![String::from("/b")]);
        check!(paths.contains("/hook"));
        check!(paths.contains("/b"));
        for other in ["/hook/", "/HOOK", "//hook", "/ho%6Fk", "/hooks", "/", ""] {
            check!(!paths.contains(other), "{other}");
        }
    }

    #[test]
    fn a_path_that_cant_match_panics() {
        for path in ["hook", "", "/hook?sign", "/hook#top"] {
            let extend = || Paths::default().extend([path]);
            check!(std::panic::catch_unwind(extend).is_err(), "{path:?}");
        }
    }
}
