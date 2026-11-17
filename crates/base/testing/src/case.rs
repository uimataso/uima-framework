use std::any::Any;
use std::fmt::Debug;
use std::future::{Future, poll_fn};
use std::panic::{AssertUnwindSafe, Location, catch_unwind};
use std::pin::pin;
use std::task::Poll;

/// Runs one case of a contract on a new context from `isolated`.
///
/// A case is an `async fn` taking `&C`, returning `()` or `Result<(), E: Debug>`. When it or
/// `isolated` panics, or it returns an error, `run_case` panics with the case's name, the line that
/// called `run_case`, and the panic message or the error.
///
/// ```
/// use uima_testing::{check, run_case};
///
/// pub async fn run_all(isolated: impl AsyncFn() -> Vec<u32>) {
///     run_case(&isolated, starts_empty).await;
/// }
///
/// async fn starts_empty(list: &Vec<u32>) {
///     check!(list.is_empty());
/// }
/// # tokio::runtime::Builder::new_current_thread().build().unwrap().block_on(run_all(async || Vec::new()));
/// ```
#[track_caller]
pub fn run_case<'a, C, R>(
    isolated: &'a impl AsyncFn() -> C,
    case: impl AsyncFn(&C) -> R + 'a,
) -> impl Future<Output = ()> + 'a
where
    C: 'a,
    R: CaseOutcome,
{
    let at = Location::caller();
    let name = case_name(&case);
    async move {
        let failure = match catch_panic(isolated()).await {
            Err(message) => Some(format!("isolated panicked: {message}")),
            Ok(ctx) => match catch_panic(case(&ctx)).await {
                Err(message) => Some(format!("panicked: {message}")),
                Ok(outcome) => outcome.into_result().err(),
            },
        };
        if let Some(failure) = failure {
            panic!("case `{name}`, run at {at}, {failure}");
        }
    }
}

/// Runs `future`, turning a panic in it into `Err` with the panic's message.
async fn catch_panic<T>(future: impl Future<Output = T>) -> Result<T, String> {
    let mut future = pin!(future);
    poll_fn(
        |cx| match catch_unwind(AssertUnwindSafe(|| future.as_mut().poll(cx))) {
            Ok(Poll::Pending) => Poll::Pending,
            Ok(Poll::Ready(output)) => Poll::Ready(Ok(output)),
            Err(payload) => Poll::Ready(Err(panic_message(&*payload).to_owned())),
        },
    )
    .await
}

/// What a case returns: `()`, or a `Result` whose error fails the case.
pub trait CaseOutcome {
    /// `Err` with a description when the case failed.
    fn into_result(self) -> Result<(), String>;
}

impl CaseOutcome for () {
    fn into_result(self) -> Result<(), String> {
        Ok(())
    }
}

impl<E: Debug> CaseOutcome for Result<(), E> {
    fn into_result(self) -> Result<(), String> {
        self.map_err(|error| format!("returned an error: {error:?}"))
    }
}

/// The case function's name without its module path or generics, such as `starts_empty`, or
/// `a closure`.
fn case_name<F>(_: &F) -> String {
    let name = std::any::type_name::<F>();
    if name.contains("{{closure}}") {
        return "a closure".to_owned();
    }
    // Drop every `<...>`, as in `passes<Ctx>` or `<Postgres as Suite>::reads_one`.
    let mut depth = 0_usize;
    let path: String = name
        .chars()
        .filter(|&c| match c {
            '<' => {
                depth += 1;
                false
            }
            '>' => {
                depth -= 1;
                false
            }
            _ => depth == 0,
        })
        .collect();
    path.rsplit("::").next().unwrap_or_default().to_owned()
}

fn panic_message(payload: &(dyn Any + Send)) -> &str {
    if let Some(message) = payload.downcast_ref::<&str>() {
        message
    } else if let Some(message) = payload.downcast_ref::<String>() {
        message
    } else {
        "a non-string payload"
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use super::*;
    use crate::{assert, check};

    struct Ctx {
        id: u32,
    }

    /// The panic message of `run`, which must panic.
    async fn panic_of(run: impl Future<Output = ()> + Send + 'static) -> String {
        let payload = tokio::spawn(run).await.unwrap_err().into_panic();
        panic_message(&*payload).to_owned()
    }

    /// `file:line:` of the line `offset` lines below the caller's.
    #[track_caller]
    fn line_below(offset: u32) -> String {
        let caller = Location::caller();
        format!("{}:{}:", caller.file(), caller.line() + offset)
    }

    async fn passes<C>(_: &C) {}

    async fn passes_with_ok<C>(_: &C) -> Result<(), String> {
        Ok(())
    }

    async fn fails_a_check(ctx: &Ctx) {
        check!(ctx.id == 0);
    }

    async fn panics(_: &Ctx) {
        panic!("boom");
    }

    async fn panics_with_a_number(_: &Ctx) {
        std::panic::panic_any(42_u8);
    }

    async fn returns_an_error(ctx: &Ctx) -> Result<(), String> {
        Err(format!("no row {}", ctx.id))
    }

    async fn fails_after_an_await(_: &Ctx) {
        tokio::task::yield_now().await;
        check!(1 + 1 == 3);
    }

    #[tokio::test]
    async fn a_passing_case_returns() {
        let isolated = async || Ctx { id: 1 };

        run_case(&isolated, passes).await;
        run_case(&isolated, passes_with_ok).await;
    }

    #[tokio::test]
    async fn each_case_gets_a_new_context() {
        let made = AtomicU32::new(0);
        let isolated = async || Ctx {
            id: made.fetch_add(1, Ordering::SeqCst),
        };

        run_case(&isolated, passes).await;
        run_case(&isolated, async |ctx: &Ctx| {
            check!(ctx.id == 1);
        })
        .await;
        check!(made.load(Ordering::SeqCst) == 2);
    }

    #[tokio::test]
    async fn a_case_that_awaits_runs_to_its_end() {
        let done = AtomicU32::new(0);
        let isolated = async || Ctx { id: 1 };

        run_case(&isolated, async |_: &Ctx| {
            tokio::task::yield_now().await;
            done.store(1, Ordering::SeqCst);
        })
        .await;
        check!(done.load(Ordering::SeqCst) == 1);
    }

    #[tokio::test]
    async fn a_failure_after_an_await_fails_the_case() {
        let message = panic_of(async {
            let isolated = async || Ctx { id: 7 };
            run_case(&isolated, fails_after_an_await).await;
        })
        .await;

        check!(message.starts_with("case `fails_after_an_await`, run at "));
    }

    #[tokio::test]
    async fn a_failing_check_names_the_case_and_the_line() {
        let at = line_below(3);
        let message = panic_of(async {
            let isolated = async || Ctx { id: 7 };
            run_case(&isolated, fails_a_check).await;
        })
        .await;

        assert!(message.starts_with("case `fails_a_check`, run at "));
        check!(message.contains(&at));
        check!(message.ends_with("panicked: check failed"));
    }

    #[tokio::test]
    async fn an_error_names_the_case_and_the_line_and_shows_the_error() {
        let at = line_below(3);
        let message = panic_of(async {
            let isolated = async || Ctx { id: 7 };
            run_case(&isolated, returns_an_error).await;
        })
        .await;

        assert!(message.starts_with("case `returns_an_error`, run at "));
        check!(message.contains(&at));
        check!(message.ends_with(r#"returned an error: "no row 7""#));
    }

    #[tokio::test]
    async fn a_panic_names_the_case_and_the_line_and_shows_its_message() {
        let at = line_below(3);
        let message = panic_of(async {
            let isolated = async || Ctx { id: 7 };
            run_case(&isolated, panics).await;
        })
        .await;

        assert!(message.starts_with("case `panics`, run at "));
        check!(message.contains(&at));
        check!(message.ends_with("panicked: boom"));
    }

    #[tokio::test]
    async fn a_non_string_panic_says_so() {
        let message = panic_of(async {
            let isolated = async || Ctx { id: 7 };
            run_case(&isolated, panics_with_a_number).await;
        })
        .await;

        check!(message.ends_with("panicked: a non-string payload"));
    }

    #[tokio::test]
    async fn a_panicking_fresh_names_the_case_and_the_line() {
        let at = line_below(3);
        let message = panic_of(async {
            let isolated = async || -> Ctx { panic!("db down") };
            run_case(&isolated, passes).await;
        })
        .await;

        assert!(message.starts_with("case `passes`, run at "));
        check!(message.contains(&at));
        check!(message.ends_with("isolated panicked: db down"));
    }

    #[test]
    fn a_case_name_drops_the_path_and_generics() {
        check!(case_name(&passes::<Ctx>) == "passes");
        check!(case_name(&fails_a_check) == "fails_a_check");
    }

    #[test]
    fn a_trait_method_case_is_named_by_its_method() {
        trait Suite {
            async fn reads_one(ctx: &Ctx);
        }
        struct Postgres;
        impl Suite for Postgres {
            async fn reads_one(_: &Ctx) {}
        }

        check!(case_name(&<Postgres as Suite>::reads_one) == "reads_one");
    }

    #[test]
    fn a_closure_case_is_named_a_closure() {
        check!(case_name(&async |_: &Ctx| {}) == "a closure");
    }
}
