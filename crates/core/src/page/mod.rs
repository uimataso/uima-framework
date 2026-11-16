//! Pagination: a [`PageRule`] checks a request's limit, and each kind of page has a query to read
//! and a page that holds what the read returns.

mod offset;

use std::fmt;

pub use offset::{CountedPage, OffsetPage, OffsetQuery};

use crate::{Invalid, Newtype};

/// How a list pages: the limit when a request gives none, and the largest it may ask for. Every
/// kind of page takes its limit from one.
///
/// ```
/// use uima_core::PageRule;
///
/// const EVENT_PAGE: PageRule = PageRule::new(20, 100);
///
/// let query = EVENT_PAGE.offset(None, Some(40)).unwrap();
/// assert_eq!((query.limit(), query.offset()), (20, 40));
/// assert!(EVENT_PAGE.offset(Some(101), None).is_err());
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
    /// When `default` is 0 or above `max`, or `max` is `u32::MAX`. In a `const`, that fails the
    /// build.
    pub const fn new(default: u32, max: u32) -> Self {
        assert!(
            0 < default && default <= max,
            "a page rule's default limit must be from 1 to its max",
        );
        assert!(
            max < u32::MAX,
            "a page rule's max must be below u32::MAX, so fetch_limit fits a u32",
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
    pub fn offset(
        &self,
        limit: Option<u32>,
        offset: Option<u32>,
    ) -> Result<OffsetQuery, Invalid<PageRule>> {
        let limit = limit.unwrap_or(self.default);
        if limit == 0 {
            return Err(Invalid::new(PageErrorKind::ZeroLimit));
        }

        if limit > self.max {
            return Err(Invalid::new(PageErrorKind::AboveMax {
                received: limit,
                max: self.max,
            }));
        }

        Ok(OffsetQuery::new(limit, offset.unwrap_or(0)))
    }
}

impl Newtype for PageRule {
    const NAME: &'static str = "Page";
    type Kind = PageErrorKind;
}

/// What is wrong with a page's limit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageErrorKind {
    /// A limit of 0.
    ZeroLimit,
    /// A limit above the rule's max.
    AboveMax { received: u32, max: u32 },
}

impl fmt::Display for PageErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ZeroLimit => write!(f, "limit can't be 0"),
            Self::AboveMax { received, max } => {
                write!(f, "limit can't exceed {max}, received: {received}")
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use uima_testing::{assert, check};

    use super::*;

    const RULE: PageRule = PageRule::new(20, 100);

    fn query(limit: u32, offset: u32) -> OffsetQuery {
        RULE.offset(Some(limit), Some(offset)).unwrap()
    }

    #[test]
    fn a_missing_limit_and_offset_take_the_default_and_zero() {
        let query = RULE.offset(None, None).unwrap();

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
        assert!(let Err(error) = RULE.offset(Some(0), None));

        check!(error.kind() == &PageErrorKind::ZeroLimit);
        check!(error.to_string() == "Page limit can't be 0");
    }

    #[test]
    fn a_limit_above_max_is_rejected() {
        assert!(let Err(error) = RULE.offset(Some(101), Some(3)));

        check!(
            error.kind()
                == &PageErrorKind::AboveMax {
                    received: 101,
                    max: 100
                }
        );
        check!(error.to_string() == "Page limit can't exceed 100, received: 101");
    }

    #[test]
    #[should_panic(expected = "default limit must be from 1 to its max")]
    fn a_rule_with_a_zero_default_panics() {
        let _ = PageRule::new(0, 100).offset(Some(5), None);
    }

    #[test]
    #[should_panic(expected = "default limit must be from 1 to its max")]
    fn a_rule_with_a_default_above_max_panics() {
        let _ = PageRule::new(101, 100).offset(Some(5), None);
    }

    #[test]
    fn max_is_the_rules_max() {
        check!(RULE.max() == 100);
    }

    #[test]
    fn a_rule_whose_default_is_its_max_is_fine() {
        check!(PageRule::new(5, 5).offset(None, None).unwrap().limit() == 5);
    }

    #[test]
    #[should_panic(expected = "max must be below u32::MAX")]
    fn a_rule_with_a_max_of_u32_max_panics() {
        let _ = PageRule::new(1, u32::MAX).offset(None, None);
    }
}
