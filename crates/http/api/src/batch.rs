//! [`distinct_uids`], for a request that names many items.

use crate::FrameworkError;

/// The distinct `uids` a request names, sorted, refusing more than `max` of them with
/// `too-many-items`. A uid named twice counts once.
pub fn distinct_uids<T: Ord>(mut uids: Vec<T>, max: usize) -> Result<Vec<T>, FrameworkError> {
    uids.sort_unstable();
    uids.dedup();
    if uids.len() > max {
        return Err(FrameworkError::TooManyItems {
            limit: max,
            actual: uids.len(),
        });
    }
    Ok(uids)
}

#[cfg(test)]
mod tests {
    use uima_testing::check;

    use super::*;

    #[test]
    fn uids_come_back_sorted_and_counted_once_each() {
        check!(distinct_uids(vec![3, 1, 3, 2, 1], 100).ok() == Some(vec![1, 2, 3]));
        check!(distinct_uids(Vec::<u32>::new(), 100).ok() == Some(vec![]));
        let full: Vec<u32> = (0..100).collect();
        check!(distinct_uids(full.clone(), 100).ok() == Some(full));

        let mut named_twice: Vec<u32> = (0..100).collect();
        named_twice.push(7);
        check!(let Ok(100) = distinct_uids(named_twice, 100).map(|uids| uids.len()));
    }

    #[test]
    fn more_distinct_uids_than_max_are_too_many_items() {
        let one_over: Vec<u32> = (0..101).collect();
        check!(let Err(FrameworkError::TooManyItems { limit: 100, actual: 101 }) = distinct_uids(one_over, 100));

        let mut uids: Vec<u32> = (0..150).collect();
        uids.extend(0..10);
        check!(let Err(FrameworkError::TooManyItems { limit: 100, actual: 150 }) = distinct_uids(uids, 100));
    }

    #[test]
    fn a_uid_that_cant_be_cloned_is_taken() {
        #[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
        struct Uid(String);

        let uids = vec![Uid("b".into()), Uid("a".into()), Uid("b".into())];

        check!(distinct_uids(uids, 2).ok() == Some(vec![Uid("a".into()), Uid("b".into())]));
    }
}
