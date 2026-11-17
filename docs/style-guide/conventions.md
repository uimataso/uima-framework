# Conventions

## Declaration order

Rust doesn't need forward declarations, so order a file top to bottom for the reader, with constants
first, then public items before private ones, and high-level logic before the supporting types and
functions it uses.

```rust
/// Length of an invite code.
pub const INVITE_CODE_LEN: usize = 8;
/// Characters an invite code is made of, shared by generating and checking a code.
const INVITE_CODE_CHARS: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

pub struct Invitation {
    code: InviteCode,
    expires_at: Timestamp,
    uses_left: u32,
}

impl Invitation {
    /// How many times one invitation can be redeemed.
    pub const MAX_USES: u32 = 10;
    const VALID_FOR: Duration = Duration::from_secs(7 * 24 * 60 * 60);

    pub fn new(now: Timestamp) -> Self { ... }
    pub fn redeem(&mut self, now: Timestamp) -> Result<(), RedeemError> { ... }
}

#[derive(Debug, thiserror::Error)]
pub enum RedeemError {
    #[error("invitation expired")]
    Expired,
    #[error("invitation used up")]
    UsedUp,
}

// private helpers, declared in the order they're first used above
fn check_not_expired(expires_at: Timestamp, now: Timestamp) -> Result<(), RedeemError> { ... }
fn take_one_use(uses_left: &mut u32) -> Result<(), RedeemError> { ... }
```

- A constant that belongs to one type is an associated constant (`Invitation::MAX_USES`), at the top
  of its `impl` block. Other constants (`INVITE_CODE_LEN`, `INVITE_CODE_CHARS`) go at the top of
  the file.
- Either way, all constants come before anything else in their block, public ones first.
- A private helper declared above the public item that uses it makes the reader hold onto context
  they can't place yet while they look for where it's used.

## Newtypes

Wrap a primitive in a newtype when the value has a meaning or a rule the primitive doesn't, such as
an ID, a count that can't go negative, or a name with a length limit. The compiler then catches a
user ID passed where an invitation ID is expected, and a value that exists is a value that was
checked.

```rust
uima_core::text!(
    /// An invitation code of `INVITE_CODE_LEN` characters from `INVITE_CODE_CHARS`.
    pub InviteCode,
    len = INVITE_CODE_LEN..=INVITE_CODE_LEN,
    chars = |c| c.is_ascii() && INVITE_CODE_CHARS.contains(&(c as u8)),
);
```

- Define it with a `uima-core` macro, `uid!` for an ID, `count!` for a count, `number!` for an
  integer within a range, `text!` for a string, and `secret!` for a string that must stay out of
  logs, such as a password or a token.
- The field is private and the only way in is a constructor that checks the rule, so every
  `InviteCode` in the program is valid. Code that takes an `InviteCode` doesn't check again.
- The constructor's error is `Invalid<InviteCode>`. Its message names the type and states the
  rule.
- Convert at the edges. Parse into the newtype where the value enters (an HTTP request, a database
  row, config) and turn it back into a primitive only where it leaves.
- Don't add `Deref` to the inner type, which hands out the primitive and loses the point.

## Time

A point in time is a `jiff::Timestamp` and a length of time is a `std::time::Duration`. jiff comes
from `uima-time`, which re-exports it so every app has the same version; an app doesn't
depend on jiff itself.

```rust
use std::time::Duration;

use uima_time::TimestampExt as _;
use uima_time::jiff::Timestamp;

fn time_left(now: Timestamp, issued_at: Timestamp, ttl: Duration) -> Duration {
    let expires_at = issued_at + ttl;
    expires_at.saturating_duration_since(now)
}
```

- The current time comes from a `Clock` in the context, through `HasClock`, never
  `Timestamp::now()`, so a test can control it.
- A timestamp plus or minus a `Duration` is jiff's own. The `Duration` between two timestamps
  comes from `TimestampExt`, so `SignedDuration` stays out of app code.
- Name a length for what it is, `token_ttl: Duration`, not `token_ttl_secs: u64`. Turn it into a
  number of seconds only where it leaves, such as a JSON field or a provider's request.
- In config, a length parses with `uima_time::parse_duration` (`30s`, `1h 30m`, `30d`) and a
  time zone with `NamedTimeZone`.

## Logging

Import the `tracing` macros a file logs with and call them by name.

```rust
use tracing::{info, warn};

info!(%url, %database, "connected to postgres");
```
