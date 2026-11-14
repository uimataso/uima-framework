//! [`Update`] and [`UpdateOption`], a patch to one field, and [`Changed`].

use std::ops::{BitOr, BitOrAssign};

/// Whether an operation actually changed something.
///
/// ```
/// use uima_core::{Changed, Update, UpdateOption};
///
/// let (mut name, mut bio) = ("uima", Some("hi"));
/// let changed = Update::Set("ui").apply(&mut name) | UpdateOption::<&str>::Keep.apply(&mut bio);
/// assert_eq!(changed, Changed(true));
/// assert_eq!((name, bio), ("ui", Some("hi")));
/// ```
#[must_use = "check whether the operation actually changed anything"]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Changed(pub bool);

/// A patch to a field that always has a value: keep it, or set a new one.
///
/// A request type reads a field that may be left out as `Option<T>`, which converts into an
/// `Update<T>`: a missing field keeps the value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Update<T> {
    #[default]
    Keep,
    Set(T),
}

/// A patch to a field that may be empty: keep it, remove it, or set a new value.
///
/// A request type reads a field that may be left out or `null` as `Option<Option<T>>`, which
/// converts into an `UpdateOption<T>`: a missing field keeps the value and a `null` removes it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum UpdateOption<T> {
    #[default]
    Keep,
    Remove,
    Set(T),
}

impl<T> Update<T> {
    /// Applies the patch to `field`; it changed unless the patch is `Keep`.
    pub fn apply(self, field: &mut T) -> Changed {
        match self {
            Self::Keep => Changed(false),
            Self::Set(value) => {
                *field = value;
                Changed(true)
            }
        }
    }

    /// Converts the value it sets.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Update<U> {
        match self {
            Self::Keep => Update::Keep,
            Self::Set(value) => Update::Set(f(value)),
        }
    }

    /// Converts the value it sets, or returns the conversion's error.
    ///
    /// ```
    /// use uima_core::Update;
    ///
    /// uima_core::number!(pub Capacity, u32, range = 2..=6);
    ///
    /// let requested: Option<u32> = Some(4);
    /// let capacity = Update::from(requested).try_map(Capacity::new).unwrap();
    /// assert_eq!(capacity, Update::Set(Capacity::new(4).unwrap()));
    /// assert!(Update::from(Some(9)).try_map(Capacity::new).is_err());
    /// ```
    pub fn try_map<U, E>(self, f: impl FnOnce(T) -> Result<U, E>) -> Result<Update<U>, E> {
        match self {
            Self::Keep => Ok(Update::Keep),
            Self::Set(value) => f(value).map(Update::Set),
        }
    }

    /// The same patch, borrowing the value it sets.
    pub fn as_ref(&self) -> Update<&T> {
        match self {
            Self::Keep => Update::Keep,
            Self::Set(value) => Update::Set(value),
        }
    }

    /// Whether the patch leaves the field as it is.
    pub fn is_keep(&self) -> bool {
        matches!(self, Self::Keep)
    }
}

impl<T> UpdateOption<T> {
    /// Applies the patch to `field`; it changed unless the patch is `Keep`, or `Remove` on an
    /// empty field.
    pub fn apply(self, field: &mut Option<T>) -> Changed {
        match self {
            Self::Keep => Changed(false),
            Self::Remove => Changed(field.take().is_some()),
            Self::Set(value) => {
                *field = Some(value);
                Changed(true)
            }
        }
    }

    /// Converts the value it sets.
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> UpdateOption<U> {
        match self {
            Self::Keep => UpdateOption::Keep,
            Self::Remove => UpdateOption::Remove,
            Self::Set(value) => UpdateOption::Set(f(value)),
        }
    }

    /// Converts the value it sets, or returns the conversion's error.
    pub fn try_map<U, E>(self, f: impl FnOnce(T) -> Result<U, E>) -> Result<UpdateOption<U>, E> {
        match self {
            Self::Keep => Ok(UpdateOption::Keep),
            Self::Remove => Ok(UpdateOption::Remove),
            Self::Set(value) => f(value).map(UpdateOption::Set),
        }
    }

    /// The same patch, borrowing the value it sets.
    pub fn as_ref(&self) -> UpdateOption<&T> {
        match self {
            Self::Keep => UpdateOption::Keep,
            Self::Remove => UpdateOption::Remove,
            Self::Set(value) => UpdateOption::Set(value),
        }
    }

    /// Whether the patch leaves the field as it is.
    pub fn is_keep(&self) -> bool {
        matches!(self, Self::Keep)
    }
}

impl BitOr for Changed {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl BitOrAssign for Changed {
    fn bitor_assign(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

impl From<bool> for Changed {
    fn from(changed: bool) -> Self {
        Self(changed)
    }
}

impl From<Changed> for bool {
    fn from(changed: Changed) -> Self {
        changed.0
    }
}

/// `None` keeps the field.
impl<T> From<Option<T>> for Update<T> {
    fn from(value: Option<T>) -> Self {
        match value {
            None => Self::Keep,
            Some(value) => Self::Set(value),
        }
    }
}

impl<T> From<Update<T>> for Option<T> {
    fn from(update: Update<T>) -> Self {
        match update {
            Update::Keep => None,
            Update::Set(value) => Some(value),
        }
    }
}

/// `None` keeps the field and `Some(None)` removes it.
impl<T> From<Option<Option<T>>> for UpdateOption<T> {
    fn from(value: Option<Option<T>>) -> Self {
        match value {
            None => Self::Keep,
            Some(None) => Self::Remove,
            Some(Some(value)) => Self::Set(value),
        }
    }
}

impl<T> From<UpdateOption<T>> for Option<Option<T>> {
    fn from(update: UpdateOption<T>) -> Self {
        match update {
            UpdateOption::Keep => None,
            UpdateOption::Remove => Some(None),
            UpdateOption::Set(value) => Some(Some(value)),
        }
    }
}

#[cfg(test)]
mod tests {
    use assert2::check;

    use super::*;

    #[test]
    fn keep_is_the_default() {
        check!(Update::<i32>::default() == Update::Keep);
        check!(UpdateOption::<i32>::default() == UpdateOption::Keep);
        check!(Changed::default() == Changed(false));
    }

    #[test]
    fn update_keep_leaves_the_field() {
        let mut field = 1;
        check!(Update::Keep.apply(&mut field) == Changed(false));
        check!(field == 1);
    }

    #[test]
    fn update_set_replaces_the_field() {
        let mut field = 1;
        check!(Update::Set(2).apply(&mut field) == Changed(true));
        check!(field == 2);
        check!(Update::Set(2).apply(&mut field) == Changed(true));
    }

    #[test]
    fn update_option_keep_leaves_the_field() {
        let mut field = Some(1);
        check!(UpdateOption::Keep.apply(&mut field) == Changed(false));
        check!(field == Some(1));
    }

    #[test]
    fn update_option_set_fills_the_field() {
        let mut field = None;
        check!(UpdateOption::Set(1).apply(&mut field) == Changed(true));
        check!(field == Some(1));
    }

    #[test]
    fn update_option_remove_changes_only_a_filled_field() {
        let mut filled = Some(1);
        check!(UpdateOption::Remove.apply(&mut filled) == Changed(true));
        check!(filled == None);

        let mut empty: Option<i32> = None;
        check!(UpdateOption::Remove.apply(&mut empty) == Changed(false));
        check!(empty == None);
    }

    #[test]
    fn map_converts_only_the_set_value() {
        check!(Update::<i32>::Keep.map(|n| n * 2) == Update::Keep);
        check!(Update::Set(1).map(|n| n * 2) == Update::Set(2));
        check!(UpdateOption::<i32>::Keep.map(|n| n * 2) == UpdateOption::Keep);
        check!(UpdateOption::<i32>::Remove.map(|n| n * 2) == UpdateOption::Remove);
        check!(UpdateOption::Set(1).map(|n| n * 2) == UpdateOption::Set(2));
    }

    #[test]
    fn try_map_converts_only_the_set_value() {
        let half = |n: i32| if n % 2 == 0 { Ok(n / 2) } else { Err("odd") };
        check!(Update::Keep.try_map(half) == Ok(Update::Keep));
        check!(Update::Set(4).try_map(half) == Ok(Update::Set(2)));
        check!(Update::Set(3).try_map(half) == Err("odd"));
        check!(UpdateOption::Keep.try_map(half) == Ok(UpdateOption::Keep));
        check!(UpdateOption::Remove.try_map(half) == Ok(UpdateOption::Remove));
        check!(UpdateOption::Set(4).try_map(half) == Ok(UpdateOption::Set(2)));
        check!(UpdateOption::Set(3).try_map(half) == Err("odd"));
    }

    #[test]
    fn as_ref_borrows_the_set_value() {
        let name = String::from("uima");
        check!(Update::Set(name.clone()).as_ref() == Update::Set(&name));
        check!(Update::<String>::Keep.as_ref() == Update::Keep);
        check!(UpdateOption::Set(name.clone()).as_ref() == UpdateOption::Set(&name));
        check!(UpdateOption::<String>::Remove.as_ref() == UpdateOption::Remove);
        check!(UpdateOption::<String>::Keep.as_ref() == UpdateOption::Keep);
    }

    #[test]
    fn is_keep_is_true_only_for_keep() {
        check!(Update::<i32>::Keep.is_keep());
        check!(!Update::Set(1).is_keep());
        check!(UpdateOption::<i32>::Keep.is_keep());
        check!(!UpdateOption::<i32>::Remove.is_keep());
        check!(!UpdateOption::Set(1).is_keep());
    }

    #[test]
    fn update_maps_to_and_from_an_option() {
        for value in [None, Some(1)] {
            let update = Update::from(value);
            check!(update.is_keep() == value.is_none());
            check!(Option::from(update) == value);
        }
    }

    #[test]
    fn update_option_maps_to_and_from_a_double_option() {
        let cases = [
            (None, UpdateOption::Keep),
            (Some(None), UpdateOption::Remove),
            (Some(Some(1)), UpdateOption::Set(1)),
        ];
        for (value, update) in cases {
            check!(UpdateOption::from(value) == update);
            check!(Option::<Option<i32>>::from(update) == value);
        }
    }

    #[test]
    fn changed_combines_with_or() {
        for (a, b) in [(false, false), (false, true), (true, false), (true, true)] {
            check!(Changed(a) | Changed(b) == Changed(a || b));
            let mut changed = Changed(a);
            changed |= Changed(b);
            check!(changed == Changed(a || b));
        }
        check!(Changed::from(true) == Changed(true));
        check!(bool::from(Changed(true)));
        check!(!bool::from(Changed(false)));
    }
}
