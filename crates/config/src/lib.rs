//! Reading a service's configuration from environment variables.
//!
//! ```
//! use std::net::SocketAddr;
//! use std::str::FromStr;
//!
//! use uima_config::{Env, EnvError};
//!
//! struct Config {
//!     addr: SocketAddr,
//!     workers: u16,
//!     data_dir: String,
//!     api_key: String,
//! }
//!
//! impl Config {
//!     fn from_env(env: &Env) -> Result<Self, EnvError> {
//!         Ok(Self {
//!             // A string default, parsed like the variable.
//!             addr: env.var("APP_ADDR").or("0.0.0.0:8080").parse(SocketAddr::from_str)?,
//!             // A typed default.
//!             workers: env.var("APP_WORKERS").parse(u16::from_str).or(4)?,
//!             data_dir: env.var("APP_DATA_DIR").or("/data").get(),
//!             api_key: env.var("APP_API_KEY").required()?,
//!         })
//!     }
//! }
//!
//! let env = Env::from_pairs([("APP_DATA_DIR", "/srv"), ("APP_API_KEY", "k3y")]);
//! let config = Config::from_env(&env).unwrap();
//! assert_eq!(config.data_dir, "/srv");
//! assert_eq!(config.workers, 4);
//! ```
//!
//! A variable set to the empty string counts as unset, since compose files often write `VAR=` to
//! mean that. Building the config from an [`Env`] rather than the process environment keeps it
//! testable: tests pass [`Env::from_pairs`] and never touch the global environment.

mod env;

pub use env::{Env, EnvError, Parsed, Var, VarOr};
