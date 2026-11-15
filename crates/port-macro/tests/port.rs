//! What the generated code does, with real ports and contexts.

use std::future::Future;
use std::sync::Arc;

use assert2::check;
use uima_port_macro::{HasPort, has_port};

#[has_port]
pub trait UserRepo {
    fn name(&self, id: u32) -> impl Future<Output = String> + Send;
}

#[has_port]
pub trait Clock {
    fn now(&self) -> u64;
}

mod ports {
    #[uima_port_macro::has_port]
    pub trait Mailer {
        fn send(&self) -> &'static str;
    }
}

struct Postgres;

impl UserRepo for Postgres {
    async fn name(&self, id: u32) -> String {
        format!("user {id}")
    }
}

impl Clock for Postgres {
    fn now(&self) -> u64 {
        1
    }
}

struct Smtp;

impl ports::Mailer for Smtp {
    fn send(&self) -> &'static str {
        "sent"
    }
}

struct FixedClock(u64);

impl Clock for FixedClock {
    fn now(&self) -> u64 {
        self.0
    }
}

#[derive(HasPort)]
struct Ctx {
    #[has_port(UserRepo, Clock)]
    postgres: Arc<Postgres>,
    #[has_port(ports::Mailer)]
    smtp: Smtp,
}

/// A context inside another, behind two `Arc`s.
#[derive(HasPort)]
struct Outer {
    #[has_port(UserRepo)]
    #[expect(clippy::redundant_allocation, reason = "tests nested `Arc`s")]
    inner: Arc<Arc<Ctx>>,
    #[has_port(Clock)]
    clock: FixedClock,
}

/// A context generic over its adapter.
#[derive(HasPort)]
struct Generic<C: Send + Sync> {
    #[has_port(Clock)]
    clock: C,
}

fn ctx() -> Ctx {
    Ctx {
        postgres: Arc::new(Postgres),
        smtp: Smtp,
    }
}

async fn name<C: HasUserRepo>(ctx: &C) -> String {
    ctx.user_repo().name(7).await
}

fn now<C: HasClock>(ctx: &C) -> u64 {
    ctx.clock().now()
}

#[tokio::test]
async fn each_port_comes_from_its_field() {
    let ctx = ctx();
    check!(name(&ctx).await == "user 7");
    check!(now(&ctx) == 1);
    check!(ports::Mailer::send(ports::HasMailer::mailer(&ctx)) == "sent");
    check!(std::ptr::eq(ctx.user_repo(), Arc::as_ptr(&ctx.postgres)));
}

#[tokio::test]
async fn a_port_passes_through_arcs_and_contexts() {
    let outer = Outer {
        inner: Arc::new(Arc::new(ctx())),
        clock: FixedClock(5),
    };
    check!(name(&outer).await == "user 7");
    check!(now(&outer) == 5);
    check!(now(&Arc::new(outer)) == 5);
    check!(
        now(&Generic {
            clock: FixedClock(9)
        }) == 9
    );
}

#[test]
fn an_adapter_provides_its_own_port() {
    check!(now(&Postgres) == 1);
    check!(now(&Arc::new(FixedClock(3))) == 3);
}

#[tokio::test]
async fn a_port_future_can_be_spawned() {
    async fn spawned<C: HasUserRepo + Send + 'static>(ctx: C) -> String {
        tokio::spawn(async move { ctx.user_repo().name(2).await })
            .await
            .expect("the task doesn't panic")
    }
    check!(spawned(Arc::new(ctx())).await == "user 2");
}

/// Two ports with the same name, at different paths.
mod a {
    #[uima_port_macro::has_port]
    pub trait Clock {
        fn now(&self) -> u64;
    }
}

mod b {
    #[uima_port_macro::has_port]
    pub trait Clock {
        fn now(&self) -> u64;
    }
}

struct Both;

impl a::Clock for Both {
    fn now(&self) -> u64 {
        10
    }
}

impl b::Clock for Both {
    fn now(&self) -> u64 {
        20
    }
}

#[derive(HasPort)]
struct Paths {
    #[has_port(a::Clock)]
    a: Both,
    #[has_port(b::Clock)]
    b: Both,
}

/// A context with its own where clause, and a field with two `has_port` attributes.
#[derive(HasPort)]
struct Bounded<C>
where
    C: Clock + UserRepo + Send + Sync,
{
    #[has_port(Clock)]
    #[has_port(UserRepo)]
    adapter: C,
}

/// A port served by a trait object.
#[derive(HasPort)]
struct Dyn {
    #[has_port(Clock)]
    clock: Arc<dyn Clock + Send + Sync>,
}

fn port_is<C: HasClock<Port = P>, P: ?Sized>(_: &C) {}

#[test]
fn the_port_is_the_adapter_type() {
    port_is::<_, Postgres>(&ctx());
    port_is::<_, FixedClock>(&Arc::new(FixedClock(1)));
    port_is::<_, dyn Clock + Send + Sync>(&Dyn {
        clock: Arc::new(FixedClock(1)),
    });
}

#[test]
fn a_port_is_send_and_sync() {
    fn send_sync<T: Send + Sync + ?Sized>(_: &T) {}
    fn get<C: HasClock>(ctx: &C) {
        send_sync(ctx.clock());
    }
    get(&ctx());
}

#[tokio::test]
async fn edge_contexts_serve_their_ports() {
    let paths = Paths { a: Both, b: Both };
    check!(a::Clock::now(a::HasClock::clock(&paths)) == 10);
    check!(b::Clock::now(b::HasClock::clock(&paths)) == 20);

    let bounded = Bounded { adapter: Postgres };
    check!(now(&bounded) == 1);
    check!(name(&bounded).await == "user 7");

    check!(
        now(&Dyn {
            clock: Arc::new(FixedClock(4))
        }) == 4
    );
    let unsized_ctx: Arc<dyn HasClock<Port = FixedClock>> = Arc::new(FixedClock(6));
    check!(now(&unsized_ctx) == 6);
}
