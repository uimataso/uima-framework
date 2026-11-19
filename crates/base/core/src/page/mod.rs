//! Pagination: a [`PageRule`] checks a request's limit, and each kind of page has a query to read
//! and a page that holds what the read returns.

mod offset;

use std::error::Error;
use std::fmt;

pub use offset::{CountedPage, OffsetPage, OffsetQuery};

/// How a list pages: the limit when a request gives none, and the largest it may ask for. Every
/// kind of page takes its limit from one.
///
/// ```
/// use uima_core::PageRule;
///
/// const EVENT_PAGE: PageRule = PageRule::new(20, 100);
///
/// let query = EVENT_PAGE.offset_query(None, Some(40)).unwrap();
/// assert_eq!((query.limit(), query.offset()), (20, 40));
/// assert!(EVENT_PAGE.offset_query(Some(101), None).is_err());
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PageRule {
    default: u32,
    max: u32,
}

impl PageRule {
    /// A rule with `default` as the limit when a request gives none and `max` as the largest it
    /// may ask for.
    ///
    /// # Panics
    ///
    /// When `default` is 0 or above `max`, or `max` is above [`OffsetQuery::MAX_LIMIT`]. In a
    /// `const`, that fails the build.
    pub const fn new(default: u32, max: u32) -> Self {
        assert!(
            0 < default && default <= max,
            "a page rule's default limit must be from 1 to its max",
        );
        assert!(
            max <= OffsetQuery::MAX_LIMIT,
            "a page rule's max must be at most OffsetQuery::MAX_LIMIT",
        );
        Self { default, max }
    }

    /// The largest limit a request may ask for, such as for an API to show its users.
    pub const fn max(self) -> u32 {
        self.max
    }

    /// The offset query a request asks for, with the default limit when it gives none and offset
    /// 0.
    ///
    /// Fails when `limit` is 0 or above the max.
    pub fn offset_query(
        &self,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<OffsetQuery, InvalidPage> {
        let limit = limit.unwrap_or(self.default);
        if limit == 0 {
            return Err(InvalidPage::ZeroLimit);
        }

        if limit > self.max {
            return Err(InvalidPage::AboveMax {
                received: limit,
                max: self.max,
            });
        }

        Ok(OffsetQuery::new(limit, offset.unwrap_or(0)))
    }
}

/// Why a [`PageRule`] refused a request's page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvalidPage {
    /// A limit of 0.
    ZeroLimit,
    /// A limit above the rule's max.
    AboveMax { received: u32, max: u32 },
}

impl fmt::Display for InvalidPage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroLimit => write!(f, "Page limit can't be 0"),
            Self::AboveMax { received, max } => {
                write!(f, "Page limit can't exceed {max}, received: {received}")
            }
        }
    }
}

impl Error for InvalidPage {}

#[cfg(test)]
mod tests {
    use uima_testing::{assert, check};

    use super::*;

    const RULE: PageRule = PageRule::new(20, 100);

    fn query(limit: u32, offset: u32) -> OffsetQuery {
        RULE.offset_query(Some(limit), Some(offset)).unwrap()
    }

    #[test]
    fn a_missing_limit_and_offset_take_the_default_and_zero() {
        let query = RULE.offset_query(None, None).unwrap();

        check!(query.limit() == 20);
        check!(query.offset() == 0);
    }

    #[test]
    fn a_limit_from_one_to_max_is_taken() {
        check!(query(1, 7).limit() == 1);
        check!(query(1, 7).offset() == 7);
        check!(query(100, u32::MAX).limit() == 100);
        check!(query(100, u32::MAX).offset() == u32::MAX);
    }

    #[test]
    fn a_zero_limit_is_rejected() {
        assert!(let Err(error) = RULE.offset_query(Some(0), None));

        check!(error == InvalidPage::ZeroLimit);
        check!(error.to_string() == "Page limit can't be 0");
    }

    #[test]
    fn a_limit_above_max_is_rejected() {
        assert!(let Err(error) = RULE.offset_query(Some(101), Some(3)));

        check!(
            error
                == InvalidPage::AboveMax {
                    received: 101,
                    max: 100
                }
        );
        check!(error.to_string() == "Page limit can't exceed 100, received: 101");
    }

    #[test]
    #[should_panic(expected = "default limit must be from 1 to its max")]
    fn a_rule_with_a_zero_default_panics() {
        let _ = PageRule::new(0, 100).offset_query(Some(5), None);
    }

    #[test]
    #[should_panic(expected = "default limit must be from 1 to its max")]
    fn a_rule_with_a_default_above_max_panics() {
        let _ = PageRule::new(101, 100).offset_query(Some(5), None);
    }

    #[test]
    fn max_is_the_rules_max() {
        check!(RULE.max() == 100);
    }

    #[test]
    fn a_rule_whose_default_is_its_max_is_fine() {
        check!(
            PageRule::new(5, 5)
                .offset_query(None, None)
                .unwrap()
                .limit()
                == 5
        );
    }

    /// The largest max is the largest limit an offset query takes.
    #[test]
    fn max_reaches_the_largest_offset_query() {
        let rule = PageRule::new(1, OffsetQuery::MAX_LIMIT);

        check!(
            rule.offset_query(Some(OffsetQuery::MAX_LIMIT), None)
                .is_ok()
        );
    }

    #[test]
    #[should_panic(expected = "max must be at most OffsetQuery::MAX_LIMIT")]
    fn a_rule_with_a_max_of_u32_max_panics() {
        let _ = PageRule::new(1, u32::MAX).offset_query(None, None);
    }
}

#[cfg(test)]
mod props {
    use proptest::prelude::*;
    use proptest::property_test;

    use super::*;

    /// Any rule, often with a max of 1 or of `OffsetQuery::MAX_LIMIT`, and a default at its edges.
    fn rule() -> impl Strategy<Value = PageRule> {
        prop_oneof![
            Just(1),
            Just(OffsetQuery::MAX_LIMIT),
            1..=OffsetQuery::MAX_LIMIT
        ]
        .prop_flat_map(|max| (prop_oneof![Just(1), Just(max), 1..=max], Just(max)))
        .prop_map(|(default, max)| PageRule::new(default, max))
    }

    /// Any limit or none, often at the edges, such as 0, 1, `max` and just past it.
    fn limit(max: u32) -> impl Strategy<Value = Option<u32>> {
        prop::option::of(prop_oneof![
            Just(0),
            Just(1),
            Just(max),
            Just(max.saturating_add(1)),
            any::<u32>(),
        ])
    }

    /// The rule's checks, spelled out.
    fn expected(
        rule: PageRule,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<OffsetQuery, InvalidPage> {
        let limit = limit.unwrap_or(rule.default);
        if limit == 0 {
            return Err(InvalidPage::ZeroLimit);
        }
        if limit > rule.max {
            return Err(InvalidPage::AboveMax {
                received: limit,
                max: rule.max,
            });
        }
        Ok(OffsetQuery::new(limit, offset.unwrap_or(0)))
    }

    /// A list of up to 30 rows, and a query of 1 to 8 rows from up to just past its end.
    fn read() -> impl Strategy<Value = (u32, OffsetQuery)> {
        (0..=30_u32, 1..=8_u32, 0..=35_u32)
            .prop_map(|(total, limit, offset)| (total, OffsetQuery::new(limit, offset)))
    }

    #[property_test]
    fn offset_query_takes_a_limit_from_one_to_max(
        #[strategy = rule().prop_flat_map(|rule| (Just(rule), limit(rule.max)))] (rule, limit): (
            PageRule,
            Option<u32>,
        ),
        #[strategy = prop::option::of(prop_oneof![Just(0), Just(u32::MAX), any::<u32>()])]
        offset: Option<u32>,
    ) {
        let got = rule.offset_query(limit, offset);
        prop_assert_eq!(got, expected(rule, limit, offset));
        if let Ok(query) = got {
            prop_assert_eq!(u64::from(query.fetch_limit()), u64::from(query.limit()) + 1);
        }
    }

    #[property_test]
    fn both_pages_show_the_rows_a_read_returns(
        #[strategy = read()] (total, query): (u32, OffsetQuery),
    ) {
        // The list's rows are 0 to total - 1, so a row is its own index.
        let rows = (query.offset()..total)
            .take(query.fetch_limit() as usize)
            .collect();
        let page = OffsetPage::from_rows(rows, query);
        let has_more = query.offset() + query.limit() < total;

        let shown: Vec<u32> = (query.offset()..total)
            .take(query.limit() as usize)
            .collect();
        prop_assert_eq!(&page.items, &shown);
        prop_assert_eq!(page.has_more, has_more);
        let counted = CountedPage {
            items: page.items,
            query,
            total: u64::from(total),
        };
        prop_assert_eq!(counted.has_more(), has_more);
    }
}
