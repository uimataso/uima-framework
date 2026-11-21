//! [`SortOrder`] and [`Sort`], which way a list is sorted and by what.

/// Which way a list is sorted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum SortOrder {
    Asc,
    Desc,
}

impl SortOrder {
    /// `"ASC"` or `"DESC"`, for an `ORDER BY`.
    pub fn as_sql(self) -> &'static str {
        match self {
            Self::Asc => "ASC",
            Self::Desc => "DESC",
        }
    }
}

/// What a list sorts by, and which way. A sort by several keys is a list of `Sort<K>`, first
/// key first.
///
/// ```
/// use uima_core::Sort;
///
/// #[derive(Clone, Copy)]
/// enum UserKey {
///     Name,
///     JoinedAt,
/// }
///
/// impl UserKey {
///     fn column(self) -> &'static str {
///         match self {
///             Self::Name => "name",
///             Self::JoinedAt => "joined_at",
///         }
///     }
/// }
///
/// let sorts = [Sort::desc(UserKey::JoinedAt), Sort::asc(UserKey::Name)];
/// let order_by: Vec<String> =
///     sorts.iter().map(|s| format!("{} {}", s.key.column(), s.order.as_sql())).collect();
/// assert_eq!(order_by.join(", "), "joined_at DESC, name ASC");
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Sort<K> {
    pub key: K,
    pub order: SortOrder,
}

impl<K> Sort<K> {
    /// Sorts by `key`, ascending.
    pub const fn asc(key: K) -> Self {
        Self {
            key,
            order: SortOrder::Asc,
        }
    }

    /// Sorts by `key`, descending.
    pub const fn desc(key: K) -> Self {
        Self {
            key,
            order: SortOrder::Desc,
        }
    }
}

#[cfg(test)]
mod tests {
    use uima_testing::check;

    use super::*;

    #[test]
    fn as_sql_is_the_keyword() {
        check!(SortOrder::Asc.as_sql() == "ASC");
        check!(SortOrder::Desc.as_sql() == "DESC");
    }

    #[test]
    fn asc_and_desc_sort_by_the_key_that_way() {
        const NEWEST: Sort<&str> = Sort::desc("created_at");
        check!(
            NEWEST
                == Sort {
                    key: "created_at",
                    order: SortOrder::Desc
                }
        );
        check!(
            Sort::asc("name")
                == Sort {
                    key: "name",
                    order: SortOrder::Asc
                }
        );
    }
}
