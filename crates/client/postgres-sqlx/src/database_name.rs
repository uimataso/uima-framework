//! [`DatabaseName`], a Postgres database's name.

uima_core::text!(
    /// A Postgres database's name, of 1 to 63 lowercase ASCII letters, digits, `_` or `-`. It fits
    /// Postgres' identifier limit, can be quoted into `CREATE DATABASE`, and has no uppercase that
    /// unquoted SQL would fold.
    pub DatabaseName,
    len = 1..=63,
    chars = |c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-',
);

#[cfg(test)]
mod tests {
    use uima_testing::check;

    use super::*;

    #[test]
    fn takes_lowercase_letters_digits_underscores_and_hyphens() {
        for name in ["nova-el", "sowah_test1", "a", &"a".repeat(63)] {
            check!(DatabaseName::new(name).is_ok(), "{name}");
        }
    }

    #[test]
    fn rejects_what_could_break_out_of_quotes_or_get_cut() {
        for name in ["", &"a".repeat(64), r#"a"b"#, "a b", "a;b", "資料"] {
            check!(DatabaseName::new(name).is_err(), "{name}");
        }
    }

    #[test]
    fn rejects_uppercase_which_unquoted_sql_would_fold() {
        for name in ["App", "A"] {
            check!(DatabaseName::new(name).is_err(), "{name}");
        }
    }
}
