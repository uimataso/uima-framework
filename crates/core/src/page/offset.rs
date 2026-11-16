//! Offset pages: [`OffsetQuery`] reads `limit` rows from `offset`, and [`OffsetPage`] or
//! [`CountedPage`] holds what the read returns.

/// Which rows of a list to read: `limit` rows from `offset`. Only
/// [`PageRule::offset`](crate::PageRule::offset) makes one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OffsetQuery {
    limit: u32,
    offset: u32,
}

impl OffsetQuery {
    pub(super) fn new(limit: u32, offset: u32) -> Self {
        Self { limit, offset }
    }

    /// How many rows the page holds at most.
    pub fn limit(self) -> u32 {
        self.limit
    }

    /// How many rows of the list come before the page.
    pub fn offset(self) -> u32 {
        self.offset
    }

    /// `limit + 1`, the `LIMIT` for a read that [`OffsetPage::from_rows`] takes, so it can tell
    /// whether more rows follow.
    pub fn fetch_limit(self) -> u32 {
        // A rule's max is below u32::MAX, so this can't overflow.
        self.limit + 1
    }
}

/// One page of a list, and whether more rows follow it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OffsetPage<T> {
    pub items: Vec<T>,
    pub query: OffsetQuery,
    pub has_more: bool,
}

impl<T> OffsetPage<T> {
    /// The page from rows read with `LIMIT query.fetch_limit()`. It drops the extra row and sets
    /// `has_more`.
    ///
    /// ```
    /// use uima_core::{OffsetPage, PageRule};
    ///
    /// const LOG_PAGE: PageRule = PageRule::new(2, 50);
    ///
    /// let query = LOG_PAGE.offset(None, None).unwrap();
    /// // a read with `LIMIT query.fetch_limit()`, which is 3
    /// let rows = vec!["a", "b", "c"];
    ///
    /// let page = OffsetPage::from_rows(rows, query);
    /// assert_eq!(page.items, ["a", "b"]);
    /// assert!(page.has_more);
    /// ```
    pub fn from_rows(mut rows: Vec<T>, query: OffsetQuery) -> Self {
        let limit = usize::try_from(query.limit).unwrap_or(usize::MAX);

        debug_assert!(
            rows.len() <= limit.saturating_add(1),
            "got {} rows for {query:?}, more rows than fetch_limit reads",
            rows.len(),
        );

        let has_more = rows.len() > limit;
        rows.truncate(limit);
        Self {
            items: rows,
            query,
            has_more,
        }
    }

    /// The same page with `f` applied to each item.
    pub fn map<U>(self, f: impl FnMut(T) -> U) -> OffsetPage<U> {
        OffsetPage {
            items: self.items.into_iter().map(f).collect(),
            query: self.query,
            has_more: self.has_more,
        }
    }
}

/// One page of a list, and how many rows the whole list has.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountedPage<T> {
    pub items: Vec<T>,
    pub query: OffsetQuery,
    pub total: u64,
}

impl<T> CountedPage<T> {
    /// Whether rows follow this page.
    pub fn has_more(&self) -> bool {
        let shown = u64::from(self.query.offset) + self.items.len() as u64;
        shown < self.total
    }

    /// The same page with `f` applied to each item.
    pub fn map<U>(self, f: impl FnMut(T) -> U) -> CountedPage<U> {
        CountedPage {
            items: self.items.into_iter().map(f).collect(),
            query: self.query,
            total: self.total,
        }
    }
}

#[cfg(test)]
mod tests {
    use uima_testing::check;

    use super::*;
    use crate::PageRule;

    fn query(limit: u32, offset: u32) -> OffsetQuery {
        PageRule::new(20, 100)
            .offset(Some(limit), Some(offset))
            .unwrap()
    }

    #[test]
    fn fetch_limit_is_one_more_than_the_limit() {
        let largest = PageRule::new(u32::MAX - 1, u32::MAX - 1);

        check!(query(1, 0).fetch_limit() == 2_u32);
        check!(largest.offset(None, None).unwrap().fetch_limit() == u32::MAX);
    }

    #[test]
    #[cfg(debug_assertions)]
    #[should_panic(expected = "more rows than fetch_limit")]
    fn rows_past_fetch_limit_are_a_bug() {
        let _ = OffsetPage::from_rows(vec![1, 2, 3, 4], query(2, 0));
    }

    #[test]
    fn rows_past_the_limit_mean_more_follow() {
        let page = OffsetPage::from_rows(vec![1, 2, 3], query(2, 4));

        check!(page.items == [1, 2]);
        check!(page.has_more);
        check!(page.query == query(2, 4));
    }

    #[test]
    fn rows_up_to_the_limit_mean_none_follow() {
        check!(
            OffsetPage::from_rows(vec![1, 2], query(2, 0))
                == OffsetPage {
                    items: vec![1, 2],
                    query: query(2, 0),
                    has_more: false
                }
        );
        check!(!OffsetPage::from_rows(vec![1], query(2, 0)).has_more);
        check!(!OffsetPage::<u8>::from_rows(vec![], query(2, 0)).has_more);
    }

    #[test]
    fn map_keeps_the_query_and_has_more() {
        let page = OffsetPage::from_rows(vec![1, 2, 3], query(2, 0)).map(|n| n * 10);

        check!(
            page == OffsetPage {
                items: vec![10, 20],
                query: query(2, 0),
                has_more: true
            }
        );
    }

    #[test]
    fn counted_has_more_while_rows_follow_the_page() {
        let counted_page = |items: Vec<u8>, offset, total| CountedPage {
            items,
            query: query(2, offset),
            total,
        };

        check!(counted_page(vec![1, 2], 0, 3).has_more());
        check!(!counted_page(vec![1, 2], 0, 2).has_more());
        check!(!counted_page(vec![3], 2, 3).has_more());
        check!(!counted_page(vec![], 0, 0).has_more());
        check!(!counted_page(vec![], 10, 3).has_more());
    }

    #[test]
    fn counted_map_keeps_the_query_and_total() {
        let page = CountedPage {
            items: vec![1, 2],
            query: query(2, 0),
            total: 9,
        }
        .map(|n| n + 1);

        check!(
            page == CountedPage {
                items: vec![2, 3],
                query: query(2, 0),
                total: 9
            }
        );
    }
}
