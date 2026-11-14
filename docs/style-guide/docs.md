# Docs

## Rustdoc

A doc comment is the caller's contract: what they need to use the item correctly, most important
first.

- Open with one sentence of what the item is or does. Many items need nothing more.
- After a blank `///` line, add only what the signature can't show: errors, edge cases, rejected
  values, and contracts such as cancellation, retries or timing.
- Describe what the caller can observe, in the caller's terms. Private helpers, constants and
  algorithms stay in the code.
- Put each rule on the item it governs: a field's rule on the field, a rule shared by siblings once
  on their parent.
- Name a parameter only when its name leaves its meaning unclear, and link another item only when
  the reader would be lost without it.
- Use precise terms ("scheduling drift", not "error"), and backtick identifiers and values.
- Add an example when it shows the use faster than prose can, such as how to call a macro or how
  types fit together. Examples run as doctests, so they stay correct.
- Reread the doc before finishing and cut whatever the caller doesn't need.

```rust
impl Invitation {
    /// Uses up one redemption of the invitation.
    ///
    /// Fails with `RedeemError::Expired` once the invitation has expired, and with
    /// `RedeemError::UsedUp` after `MAX_USES` redemptions.
    pub fn redeem(&mut self, now: Timestamp) -> Result<(), RedeemError> { ... }
}
```

## Markdown

A doc under `docs/` states the rule and points at where the details live.

- Leave out what will go out of date soon, what is more specific than the reader needs, and what a
  file already holds: link the `justfile`, `Cargo.toml` or config instead of copying it.
- Say each thing once, in the section it belongs to, and link it from elsewhere.
- Don't list the things a rule applies to, such as which services use a tool; the list goes stale.
- Give a rule's reason when it helps apply the rule, in a clause. How a decision was reached goes
  in the commit message.

## Text

This covers comments, docs, commit messages and PR descriptions. Unicode is fine where it means
something, such as a name, a math symbol or a diagram, but not as decoration:

- No emoji.
- No em or en dashes, and no `--` in a sentence: a comma, colon, semicolon or parentheses reads
  better.
- Straight quotes and `...`, not curly quotes or the ellipsis character.
- Wrap at 100 columns, like the code.
