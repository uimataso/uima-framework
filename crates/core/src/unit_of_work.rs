//! [`UnitOfWork`], several writes that commit or roll back together.

use std::ops::Deref;

use crate::Failure;

/// Several writes through one context's ports that commit or roll back together, such as a
/// database transaction.
///
/// Implementations roll back when dropped without [`commit`](Self::commit), so a usecase that
/// returns early with `?` leaves nothing behind.
///
/// A write cancelled after it was sent may still run, so roll back a unit of work whose write was
/// cancelled instead of committing it.
pub trait UnitOfWork: Send + Sync {
    /// Keeps every write made through it. When it fails or is cancelled, the writes may have been
    /// kept, so a caller that retries makes its writes safe to apply twice.
    fn commit(self) -> impl Future<Output = Result<(), Failure>> + Send;

    /// Discards every write made through it.
    fn rollback(self) -> impl Future<Output = Result<(), Failure>> + Send;
}

/// A context that starts units of work.
///
/// Writes through the context itself run outside its units of work, so one made while a unit of
/// work is open can wait on that unit, such as on a row it locked or the connection it holds. End
/// the unit of work first.
///
/// A usecase names the ports it writes through on the unit of work as bounds on `Uow`:
///
/// ```
/// use uima_core::HasUnitOfWork;
/// # trait HasGroupRepo {}
/// # trait HasMemberRepo {}
///
/// async fn create_group<C>(ctx: &C) -> Result<(), uima_core::Failure>
/// where
///     C: HasUnitOfWork<Uow: HasGroupRepo + HasMemberRepo>,
/// {
///     let uow = ctx.uow("create group").await?;
///     // uow.group_repo().create(..), uow.member_repo().add(..)
///     uow.commit().await
/// }
/// ```
pub trait HasUnitOfWork: Send + Sync {
    type Uow: UnitOfWork;

    /// Starts a unit of work. Adapters implement it; usecases call [`uow`](Self::uow).
    fn begin(&self) -> impl Future<Output = Result<Self::Uow, Failure>> + Send;

    /// Starts a unit of work whose begin, commit and rollback errors name `operation`, such as
    /// `"create group"`.
    fn uow(
        &self,
        operation: &'static str,
    ) -> impl Future<Output = Result<ScopedUow<Self::Uow>, Failure>> + Send {
        async move {
            let inner = self
                .begin()
                .await
                .map_err(|error| error.context(format!("failed to begin {operation}")))?;
            Ok(ScopedUow { inner, operation })
        }
    }
}

/// A unit of work whose begin, commit and rollback errors name the operation it was started for.
/// It reaches the unit of work's ports through `Deref`.
#[derive(Debug)]
pub struct ScopedUow<U> {
    inner: U,
    operation: &'static str,
}

impl<U: UnitOfWork> ScopedUow<U> {
    /// [`UnitOfWork::commit`], with an error naming the operation.
    pub async fn commit(self) -> Result<(), Failure> {
        let operation = self.operation;
        self.inner
            .commit()
            .await
            .map_err(|error| error.context(format!("failed to commit {operation}")))
    }

    /// [`UnitOfWork::rollback`], with an error naming the operation.
    pub async fn rollback(self) -> Result<(), Failure> {
        let operation = self.operation;
        self.inner
            .rollback()
            .await
            .map_err(|error| error.context(format!("failed to roll back {operation}")))
    }
}

impl<U> Deref for ScopedUow<U> {
    type Target = U;

    fn deref(&self) -> &U {
        &self.inner
    }
}

/// The cases a [`HasUnitOfWork`] passes, run by each adapter on a context that also implements
/// [`Probe`](contract::Probe). Only an adapter isolating its units of work also runs
/// [`concurrency`](contract::concurrency).
#[cfg(feature = "testing")]
pub mod contract {
    use uima_testing::{check, run_case};

    use super::*;
    use crate::Uuid;

    /// Writes and finds a row by key, so a case can tell which writes a unit of work kept. An
    /// adapter implements it for its context and its unit of work, such as over a test table.
    pub trait Probe {
        /// Writes a row with `key`, failing when one is already there.
        fn write(&self, key: Uuid) -> impl Future<Output = anyhow::Result<()>> + Send;

        /// Whether a row with `key` is there.
        fn exists(&self, key: Uuid) -> impl Future<Output = anyhow::Result<bool>> + Send;
    }

    /// Runs every case, each on its own context from `isolated`.
    pub async fn run_all<C>(isolated: impl AsyncFn() -> C)
    where
        C: HasUnitOfWork<Uow: Probe> + Probe,
    {
        run_case(&isolated, commit_keeps_writes).await;
        run_case(&isolated, drop_without_commit_discards_writes).await;
        run_case(&isolated, rollback_discards_writes).await;
        run_case(&isolated, a_dropped_units_keys_are_writable_again).await;
        run_case(
            &isolated,
            commit_after_a_failed_write_fails_or_keeps_the_writes,
        )
        .await;
    }

    pub async fn commit_keeps_writes<C>(ctx: &C) -> anyhow::Result<()>
    where
        C: HasUnitOfWork<Uow: Probe> + Probe,
    {
        let key = Uuid::now_v7();
        let uow = ctx.uow("contract").await?;
        uow.write(key).await?;
        uow.commit().await?;

        check!(ctx.exists(key).await?);
        Ok(())
    }

    pub async fn drop_without_commit_discards_writes<C>(ctx: &C) -> anyhow::Result<()>
    where
        C: HasUnitOfWork<Uow: Probe> + Probe,
    {
        let key = Uuid::now_v7();
        let uow = ctx.uow("contract").await?;
        uow.write(key).await?;
        drop(uow);

        check!(!ctx.exists(key).await?);
        Ok(())
    }

    pub async fn rollback_discards_writes<C>(ctx: &C) -> anyhow::Result<()>
    where
        C: HasUnitOfWork<Uow: Probe> + Probe,
    {
        let key = Uuid::now_v7();
        let uow = ctx.uow("contract").await?;
        uow.write(key).await?;
        uow.rollback().await?;

        check!(!ctx.exists(key).await?);
        ctx.write(key).await?;
        Ok(())
    }

    pub async fn a_dropped_units_keys_are_writable_again<C>(ctx: &C) -> anyhow::Result<()>
    where
        C: HasUnitOfWork<Uow: Probe> + Probe,
    {
        let key = Uuid::now_v7();
        let uow = ctx.uow("contract").await?;
        uow.write(key).await?;
        drop(uow);
        ctx.write(key).await?;

        check!(ctx.exists(key).await?);
        Ok(())
    }

    pub async fn commit_after_a_failed_write_fails_or_keeps_the_writes<C>(
        ctx: &C,
    ) -> anyhow::Result<()>
    where
        C: HasUnitOfWork<Uow: Probe> + Probe,
    {
        let key = Uuid::now_v7();
        let uow = ctx.uow("contract").await?;
        uow.write(key).await?;
        check!(uow.write(key).await.is_err());

        if uow.commit().await.is_ok() {
            check!(ctx.exists(key).await?);
        }
        Ok(())
    }

    /// Cases that overlap a unit of work with other writes, which only an adapter isolating
    /// them, such as a database, passes.
    pub mod concurrency {
        use uima_testing::{check, run_case};

        use super::Probe;
        use crate::Uuid;
        use crate::unit_of_work::HasUnitOfWork;

        /// Runs every case, each on its own context from `isolated`.
        pub async fn run_all<C>(isolated: impl AsyncFn() -> C)
        where
            C: HasUnitOfWork<Uow: Probe> + Probe,
        {
            run_case(&isolated, uncommitted_writes_are_unseen_outside).await;
            run_case(&isolated, rollback_keeps_writes_made_outside).await;
            run_case(&isolated, rollback_keeps_another_units_commit).await;
        }

        pub async fn uncommitted_writes_are_unseen_outside<C>(ctx: &C) -> anyhow::Result<()>
        where
            C: HasUnitOfWork<Uow: Probe> + Probe,
        {
            let key = Uuid::now_v7();
            let uow = ctx.uow("contract").await?;
            check!(!uow.exists(key).await?);
            uow.write(key).await?;

            check!(uow.exists(key).await?);
            check!(!ctx.exists(key).await?);
            uow.commit().await?;
            check!(ctx.exists(key).await?);
            Ok(())
        }

        pub async fn rollback_keeps_writes_made_outside<C>(ctx: &C) -> anyhow::Result<()>
        where
            C: HasUnitOfWork<Uow: Probe> + Probe,
        {
            let (inside, outside) = (Uuid::now_v7(), Uuid::now_v7());
            let uow = ctx.uow("contract").await?;
            uow.write(inside).await?;
            ctx.write(outside).await?;
            uow.rollback().await?;

            check!(!ctx.exists(inside).await?);
            check!(ctx.exists(outside).await?);
            Ok(())
        }

        pub async fn rollback_keeps_another_units_commit<C>(ctx: &C) -> anyhow::Result<()>
        where
            C: HasUnitOfWork<Uow: Probe> + Probe,
        {
            let (ours, theirs) = (Uuid::now_v7(), Uuid::now_v7());
            let uow = ctx.uow("contract").await?;
            uow.write(ours).await?;
            let other = ctx.uow("contract").await?;
            other.write(theirs).await?;
            other.commit().await?;
            uow.rollback().await?;

            check!(!ctx.exists(ours).await?);
            check!(ctx.exists(theirs).await?);
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use uima_testing::check;

    use super::*;
    use crate::FailureKind;

    /// A context whose units of work fail with `begin` when starting and `end` when ending.
    #[derive(Debug)]
    struct Failing {
        begin: Option<FailureKind>,
        end: Option<FailureKind>,
    }

    #[derive(Debug)]
    struct FailingUow {
        end: Option<FailureKind>,
    }

    fn fail(kind: Option<FailureKind>) -> Result<(), Failure> {
        match kind {
            Some(kind) => Err(Failure::new(kind, anyhow::anyhow!("boom"))),
            None => Ok(()),
        }
    }

    impl HasUnitOfWork for Failing {
        type Uow = FailingUow;

        async fn begin(&self) -> Result<FailingUow, Failure> {
            fail(self.begin)?;
            Ok(FailingUow { end: self.end })
        }
    }

    impl UnitOfWork for FailingUow {
        async fn commit(self) -> Result<(), Failure> {
            fail(self.end)
        }

        async fn rollback(self) -> Result<(), Failure> {
            fail(self.end)
        }
    }

    #[tokio::test]
    async fn errors_name_the_operation_and_keep_their_kind() {
        let ctx = Failing {
            begin: Some(FailureKind::Timeout),
            end: None,
        };
        let error = ctx.uow("create group").await.unwrap_err();
        check!(error.to_string() == "failed to begin create group");
        check!(error.kind() == FailureKind::Timeout);

        let ctx = Failing {
            begin: None,
            end: Some(FailureKind::Conflict),
        };
        let error = ctx
            .uow("create group")
            .await
            .unwrap()
            .commit()
            .await
            .unwrap_err();
        check!(error.to_string() == "failed to commit create group");
        check!(error.kind() == FailureKind::Conflict);

        let error = ctx
            .uow("create group")
            .await
            .unwrap()
            .rollback()
            .await
            .unwrap_err();
        check!(error.to_string() == "failed to roll back create group");
        check!(error.kind() == FailureKind::Conflict);
    }

    /// The contract against a unit of work in memory, whole or broken one way.
    #[cfg(feature = "testing")]
    mod with_the_contract {
        use std::collections::HashSet;
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{Arc, Mutex};

        use uima_testing::check;

        use super::*;
        use crate::Uuid;
        use crate::unit_of_work::contract::{self, Probe, concurrency};

        /// One way a unit of work breaks its contract.
        #[derive(Clone, Copy, PartialEq)]
        enum Break {
            CommitDiscards,
            RollbackKeeps,
            RollbackLocks,
            DropKeeps,
            DropLocks,
            FailedWriteDiscards,
        }

        /// Rows in memory. A unit of work writes through to them and puts back the rows it
        /// started from unless committed, so it doesn't isolate its writes; `breaks` breaks it
        /// one more way.
        #[derive(Default, Clone)]
        struct Memory {
            rows: Arc<Mutex<HashSet<Uuid>>>,
            /// Keys an ended unit of work still holds.
            locked: Arc<Mutex<HashSet<Uuid>>>,
            breaks: Option<Break>,
        }

        struct MemoryUow {
            memory: Memory,
            start: Option<HashSet<Uuid>>,
            written: Mutex<Vec<Uuid>>,
            failed: AtomicBool,
        }

        impl Probe for Memory {
            async fn write(&self, key: Uuid) -> anyhow::Result<()> {
                anyhow::ensure!(
                    !self.locked.lock().unwrap().contains(&key),
                    "{key} is locked"
                );
                anyhow::ensure!(
                    self.rows.lock().unwrap().insert(key),
                    "{key} is already there"
                );
                Ok(())
            }

            async fn exists(&self, key: Uuid) -> anyhow::Result<bool> {
                Ok(self.rows.lock().unwrap().contains(&key))
            }
        }

        impl Probe for MemoryUow {
            async fn write(&self, key: Uuid) -> anyhow::Result<()> {
                let written = self.memory.write(key).await;
                match written {
                    Ok(()) => self.written.lock().unwrap().push(key),
                    Err(_) => self.failed.store(true, Ordering::Relaxed),
                }
                written
            }

            async fn exists(&self, key: Uuid) -> anyhow::Result<bool> {
                self.memory.exists(key).await
            }
        }

        impl HasUnitOfWork for Memory {
            type Uow = MemoryUow;

            async fn begin(&self) -> Result<MemoryUow, Failure> {
                Ok(MemoryUow {
                    memory: self.clone(),
                    start: Some(self.rows.lock().unwrap().clone()),
                    written: Mutex::default(),
                    failed: AtomicBool::new(false),
                })
            }
        }

        impl MemoryUow {
            fn breaks(&self, way: Break) -> bool {
                self.memory.breaks == Some(way)
            }

            fn put_back(&mut self) {
                if let Some(start) = self.start.take() {
                    *self.memory.rows.lock().unwrap() = start;
                }
            }

            /// Keeps the keys it wrote locked after it ends.
            fn lock_written(&self) {
                let written = self.written.lock().unwrap();
                self.memory.locked.lock().unwrap().extend(written.iter());
            }
        }

        impl UnitOfWork for MemoryUow {
            async fn commit(mut self) -> Result<(), Failure> {
                let failed = self.failed.load(Ordering::Relaxed);
                if self.breaks(Break::CommitDiscards)
                    || self.breaks(Break::FailedWriteDiscards) && failed
                {
                    self.put_back();
                }
                self.start = None;
                Ok(())
            }

            async fn rollback(mut self) -> Result<(), Failure> {
                if !self.breaks(Break::RollbackKeeps) {
                    self.put_back();
                }
                if self.breaks(Break::RollbackLocks) {
                    self.lock_written();
                }
                self.start = None;
                Ok(())
            }
        }

        impl Drop for MemoryUow {
            fn drop(&mut self) {
                if self.start.is_none() || self.breaks(Break::DropKeeps) {
                    return;
                }
                self.put_back();
                if self.breaks(Break::DropLocks) {
                    self.lock_written();
                }
            }
        }

        /// Whether `case` fails on a context whose units of work break as `breaks` says.
        async fn fails<F>(breaks: Option<Break>, case: impl FnOnce(Memory) -> F) -> bool
        where
            F: Future<Output = anyhow::Result<()>> + Send + 'static,
        {
            tokio::spawn(case(Memory {
                breaks,
                ..Memory::default()
            }))
            .await
            .map_or(true, |result| result.is_err())
        }

        #[tokio::test]
        async fn a_whole_unit_of_work_passes() {
            contract::run_all(async || Memory::default()).await;
        }

        #[tokio::test]
        async fn a_broken_unit_of_work_fails() {
            let ran = tokio::spawn(contract::run_all(async || Memory {
                breaks: Some(Break::DropKeeps),
                ..Memory::default()
            }))
            .await;

            check!(ran.is_err_and(|error| error.is_panic()));
        }

        #[tokio::test]
        async fn units_of_work_writing_through_fail_the_concurrency_cases() {
            let ran = tokio::spawn(concurrency::run_all(async || Memory::default())).await;

            check!(ran.is_err_and(|error| error.is_panic()));
        }

        #[tokio::test]
        async fn each_case_fails_the_unit_of_work_breaking_it() {
            use Break::*;

            check!(
                fails(Some(CommitDiscards), |ctx| async move {
                    contract::commit_keeps_writes(&ctx).await
                })
                .await
            );
            check!(
                fails(Some(DropKeeps), |ctx| async move {
                    contract::drop_without_commit_discards_writes(&ctx).await
                })
                .await
            );
            check!(
                fails(Some(RollbackKeeps), |ctx| async move {
                    contract::rollback_discards_writes(&ctx).await
                })
                .await
            );
            check!(
                fails(Some(RollbackLocks), |ctx| async move {
                    contract::rollback_discards_writes(&ctx).await
                })
                .await
            );
            check!(
                fails(Some(DropLocks), |ctx| async move {
                    contract::a_dropped_units_keys_are_writable_again(&ctx).await
                })
                .await
            );
            check!(
                fails(Some(FailedWriteDiscards), |ctx| async move {
                    contract::commit_after_a_failed_write_fails_or_keeps_the_writes(&ctx).await
                })
                .await
            );
            // A unit of work in memory writes through, so it fails every isolation case.
            check!(
                fails(None, |ctx| async move {
                    concurrency::uncommitted_writes_are_unseen_outside(&ctx).await
                })
                .await
            );
            check!(
                fails(None, |ctx| async move {
                    concurrency::rollback_keeps_writes_made_outside(&ctx).await
                })
                .await
            );
            check!(
                fails(None, |ctx| async move {
                    concurrency::rollback_keeps_another_units_commit(&ctx).await
                })
                .await
            );
        }
    }

    #[test]
    fn a_generic_usecase_is_send() {
        fn is_send(_: impl Future + Send) {}
        fn usecase<C: HasUnitOfWork>(ctx: &C) {
            is_send(async {
                let uow = ctx.uow("create group").await?;
                let reached = &*uow;
                std::future::ready(()).await;
                let _ = reached;
                uow.commit().await
            });
            is_send(async { ctx.uow("create group").await?.rollback().await });
        }
        usecase(&Failing {
            begin: None,
            end: None,
        });
    }

    #[tokio::test]
    async fn a_scoped_unit_of_work_reaches_the_inner_one() {
        let ctx = Failing {
            begin: None,
            end: Some(FailureKind::Other),
        };
        let uow = ctx.uow("create group").await.unwrap();

        check!(uow.end == Some(FailureKind::Other));
    }
}
