//! [`MaxConnections`], how many connections a pool opens at most.

use std::fmt;
use std::str::FromStr;

use uima_core::Invalid;

uima_core::number!(
    /// How many connections a pool opens at most, from 1 to 1000. Every pool on a server shares
    /// its connection limit, which is 100 by default.
    pub MaxConnections, u32, range = 1..=1000
);

impl FromStr for MaxConnections {
    type Err = InvalidMaxConnections;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let number: i64 = s
            .trim()
            .parse()
            .map_err(|_| InvalidMaxConnections::NotANumber)?;
        Self::try_from(number).map_err(InvalidMaxConnections::OutOfRange)
    }
}

/// Why a string isn't a [`MaxConnections`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum InvalidMaxConnections {
    /// Not a whole number.
    NotANumber,
    /// A number outside 1 to 1000.
    OutOfRange(Invalid<MaxConnections>),
}

impl fmt::Display for InvalidMaxConnections {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotANumber => write!(f, "MaxConnections isn't a number"),
            Self::OutOfRange(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for InvalidMaxConnections {}

#[cfg(test)]
mod tests {
    use uima_testing::{assert, check};

    use super::*;

    #[test]
    fn parses_a_number_in_range() {
        check!("1".parse::<MaxConnections>().map(MaxConnections::get) == Ok(1));
        check!("1000".parse::<MaxConnections>().map(MaxConnections::get) == Ok(1000));
        check!(" 20 ".parse::<MaxConnections>().map(MaxConnections::get) == Ok(20));
    }

    #[test]
    fn rejects_a_number_out_of_range() {
        for s in ["0", "1001", "4294967295", "-1"] {
            assert!(let Err(InvalidMaxConnections::OutOfRange(_)) = s.parse::<MaxConnections>(), "{s}");
        }
        check!(
            "0".parse::<MaxConnections>().unwrap_err().to_string()
                == "MaxConnections must be between 1 and 1000, received: 0"
        );
    }

    #[test]
    fn rejects_what_isnt_a_number() {
        for s in ["", "many", "1.5"] {
            check!(
                s.parse::<MaxConnections>() == Err(InvalidMaxConnections::NotANumber),
                "{s}"
            );
        }
        check!(
            "many".parse::<MaxConnections>().unwrap_err().to_string()
                == "MaxConnections isn't a number"
        );
    }
}
