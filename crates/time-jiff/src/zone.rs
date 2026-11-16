use std::fmt;
use std::str::FromStr;

use jiff::tz::TimeZone;

/// A time zone from the tz database, with its name there, such as `Asia/Taipei`.
///
/// The lookup ignores case, and the name keeps the database's spelling, so `asia/taipei` gives
/// `Asia/Taipei`. Two are equal when their names are.
///
/// ```
/// use uima_time_jiff::NamedTimeZone;
///
/// let tz: NamedTimeZone = "Asia/Taipei".parse().unwrap();
/// assert_eq!(tz.name(), "Asia/Taipei");
/// ```
#[derive(Clone)]
pub struct NamedTimeZone {
    name: String,
    zone: TimeZone,
}

impl NamedTimeZone {
    /// The zone named `name` in the tz database.
    pub fn new(name: impl Into<String>) -> Result<Self, InvalidTimeZone> {
        let mut name = name.into();
        let zone = jiff::tz::db().get(&name).map_err(|_| InvalidTimeZone)?;
        if let Some(canonical) = zone.iana_name() {
            canonical.clone_into(&mut name);
        }
        Ok(Self { name, zone })
    }

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn zone(&self) -> &TimeZone {
        &self.zone
    }
}

impl FromStr for NamedTimeZone {
    type Err = InvalidTimeZone;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        Self::new(s)
    }
}

impl PartialEq for NamedTimeZone {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}

impl Eq for NamedTimeZone {}

impl fmt::Debug for NamedTimeZone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("NamedTimeZone").field(&self.name).finish()
    }
}

impl fmt::Display for NamedTimeZone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)
    }
}

#[cfg(feature = "serde")]
impl ::serde::Serialize for NamedTimeZone {
    fn serialize<S: ::serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.name)
    }
}

#[cfg(feature = "serde")]
impl<'de> ::serde::Deserialize<'de> for NamedTimeZone {
    fn deserialize<D: ::serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let name = String::deserialize(deserializer)?;
        Self::new(name).map_err(::serde::de::Error::custom)
    }
}

/// [`NamedTimeZone::new`] got a name that isn't in the tz database. The message leaves the name
/// out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InvalidTimeZone;

impl fmt::Display for InvalidTimeZone {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("isn't a time zone in the tz database, such as `Asia/Taipei`")
    }
}

impl std::error::Error for InvalidTimeZone {}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use uima_testing::check;

    use super::*;

    #[test]
    fn a_tz_name_resolves_to_its_zone() {
        let tz = NamedTimeZone::new("Asia/Taipei").unwrap();
        let noon_utc: Timestamp = "2026-01-01T12:00:00Z".parse().unwrap();

        check!(tz.name() == "Asia/Taipei");
        check!(tz.to_string() == "Asia/Taipei");
        check!(noon_utc.to_zoned(tz.zone().clone()).hour() == 20);
    }

    #[test]
    fn utc_is_a_name_too() {
        check!(NamedTimeZone::new("UTC").unwrap().zone() == &TimeZone::UTC);
    }

    #[test]
    fn an_unknown_name_is_refused() {
        for name in ["", " ", "Asia/Nowhere", " Asia/Taipei", "+08:00"] {
            check!(NamedTimeZone::new(name) == Err(InvalidTimeZone), "{name:?}");
        }
    }

    #[test]
    fn the_name_is_the_tz_database_spelling() {
        let tz = NamedTimeZone::new("asia/taipei").unwrap();

        check!(tz.name() == "Asia/Taipei");
        check!(tz == NamedTimeZone::new("Asia/Taipei").unwrap());
    }

    #[test]
    fn the_error_says_what_a_name_looks_like() {
        check!(InvalidTimeZone.to_string().contains("Asia/Taipei"));
    }

    #[test]
    fn equal_by_name() {
        let a: NamedTimeZone = "Asia/Taipei".parse().unwrap();
        let b: NamedTimeZone = "Asia/Taipei".parse().unwrap();
        let c: NamedTimeZone = "Asia/Tokyo".parse().unwrap();

        check!(a == b);
        check!(a != c);
    }

    #[test]
    fn debug_shows_the_name() {
        check!(format!("{:?}", NamedTimeZone::new("UTC").unwrap()) == r#"NamedTimeZone("UTC")"#);
    }

    #[cfg(feature = "serde")]
    #[test]
    fn serde_round_trips_the_name() {
        let tz: NamedTimeZone = serde_json::from_str(r#""Asia/Taipei""#).unwrap();

        check!(tz.name() == "Asia/Taipei");
        check!(serde_json::to_string(&tz).unwrap() == r#""Asia/Taipei""#);
        check!(serde_json::from_str::<NamedTimeZone>(r#""Asia/Nowhere""#).is_err());
    }
}
