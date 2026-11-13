//! Reading a service's configuration from environment variables.
//!
//! ```
//! use std::net::SocketAddr;
//!
//! use uima_config::{Env, EnvError};
//!
//! struct Config {
//!     addr: SocketAddr,
//!     data_dir: String,
//! }
//!
//! impl Config {
//!     fn from_env(env: &Env) -> Result<Self, EnvError> {
//!         Ok(Self {
//!             addr: env.parse_or("APP_ADDR", SocketAddr::from(([0, 0, 0, 0], 8080)))?,
//!             data_dir: env.get_or("APP_DATA_DIR", "/data"),
//!         })
//!     }
//! }
//!
//! let config = Config::from_env(&Env::from_pairs([("APP_DATA_DIR", "/srv")])).unwrap();
//! assert_eq!(config.data_dir, "/srv");
//! ```
//!
//! A variable set to the empty string counts as unset, since compose files often write `VAR=` to
//! mean that. Building the config from an [`Env`] rather than the process environment keeps it
//! testable: tests pass [`Env::from_pairs`] and never touch the global environment.

mod env;

pub use env::{Env, EnvError};
