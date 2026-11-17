//! [`UnitOfWork`], several writes that commit or roll back together.

use std::ops::Deref;

use crate::Failure;

/// Several writes through one context's ports that commit or roll back together, such as a
/// database transaction.
///
/// Implementations roll back when dropped without [`commit`](Self::commit), so a usecase that
/// returns early with `?` leaves nothing behind.
pub trait UnitOfWork: Send + Sync {
    /// Keeps every write made through it.
    fn commit(self) -> impl Future<Output = Result<(), Failure>> + Send;

    /// Discards every write made through it.
    fn rollback(self) -> impl Future<Output = Result<(), Failure>> + Send;
}

/// A context that starts units of work.
///
/// A usecase names the ports it writes through on the unit of work as bounds on `UoW`:
///
/// ```
/// use uima_core::unit_of_work::HasUnitOfWork;
/// # trait HasGroupRepo {}
/// # trait HasMemberRepo {}
///
/// async fn create_group<C>(ctx: &C) -> Result<(), uima_core::Failure>
/// where
///     C: HasUnitOfWork<UoW: HasGroupRepo + HasMemberRepo>,
/// {
///     let uow = ctx.uow("create group").await?;
///     // uow.group_repo().create(..), uow.member_repo().add(..)
///     uow.commit().await
/// }
/// ```
pub trait HasUnitOfWork: Send + Sync {
    type UoW: UnitOfWork;

    /// Starts a unit of work. Adapters implement it; usecases call [`uow`](Self::uow).
    fn begin(&self) -> impl Future<Output = Result<Self::UoW, Failure>> + Send;

    /// Starts a unit of work whose errors name `operation`, such as `"create group"`.
    fn uow(
        &self,
        operation: &'static str,
    ) -> impl Future<Output = Result<ScopedUow<Self::UoW>, Failure>> + Send {
        async move {
            let inner = self
                .begin()
                .await
                .map_err(|error| error.context(format!("failed to begin {operation}")))?;
            Ok(ScopedUow { inner, operation })
        }
    }
}

/// A unit of work whose errors name the operation it was started for. It reaches the unit of
/// work's ports through `Deref`.
pub struct ScopedUow<U> {
    inner: U,
    operation: &'static str,
}

impl<U: UnitOfWork> ScopedUow<U> {
    /// Keeps every write made through it.
    pub async fn commit(self) -> Result<(), Failure> {
        let operation = self.operation;
        self.inner
            .commit()
            .await
            .map_err(|error| error.context(format!("failed to commit {operation}")))
    }

    /// Discards every write made through it.
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

/// The cases every [`HasUnitOfWork`] passes, run by each adapter on a context that also
/// implements [`Probe`](contract::Probe).
#[cfg(feature = "testing")]
pub mod contract {
    use uima_testing::{check, run_case};

    use super::*;
    use crate::Uuid;

    /// Writes and finds a row by key, so a case can tell which writes a unit of work kept. An
    /// adapter implements it for its context and its unit of work, such as over a test table.
    pub trait Probe: Send + Sync {
        /// Writes a row with `key`.
        fn write(&self, key: Uuid) -> impl Future<Output = anyhow::Result<()>> + Send;

        /// Whether a row with `key` is there.
        fn exists(&self, key: Uuid) -> impl Future<Output = anyhow::Result<bool>> + Send;
    }

    /// Runs every case, each on its own context from `isolated`.
    pub async fn run_all<C>(isolated: impl AsyncFn() -> C)
    where
        C: HasUnitOfWork<UoW: Probe> + Probe,
    {
        run_case(&isolated, commit_keeps_writes).await;
        run_case(&isolated, drop_without_commit_discards_writes).await;
        run_case(&isolated, rollback_discards_writes).await;
        run_case(&isolated, a_dropped_units_keys_are_writable_again).await;
    }

    pub async fn commit_keeps_writes<C>(ctx: &C) -> anyhow::Result<()>
    where
        C: HasUnitOfWork<UoW: Probe> + Probe,
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
        C: HasUnitOfWork<UoW: Probe> + Probe,
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
        C: HasUnitOfWork<UoW: Probe> + Probe,
    {
        let key = Uuid::now_v7();
        let uow = ctx.uow("contract").await?;
        uow.write(key).await?;
        uow.rollback().await?;

        check!(!ctx.exists(key).await?);
        Ok(())
    }

    pub async fn a_dropped_units_keys_are_writable_again<C>(ctx: &C) -> anyhow::Result<()>
    where
        C: HasUnitOfWork<UoW: Probe> + Probe,
    {
        let key = Uuid::now_v7();
        let uow = ctx.uow("contract").await?;
        uow.write(key).await?;
        drop(uow);
        ctx.write(key).await?;

        check!(ctx.exists(key).await?);
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
            C: HasUnitOfWork<UoW: Probe> + Probe,
        {
            run_case(&isolated, uncommitted_writes_are_unseen_outside).await;
            run_case(&isolated, rollback_keeps_writes_made_outside).await;
            run_case(&isolated, rollback_keeps_another_units_commit).await;
        }

        pub async fn uncommitted_writes_are_unseen_outside<C>(ctx: &C) -> anyhow::Result<()>
        where
            C: HasUnitOfWork<UoW: Probe> + Probe,
        {
            let key = Uuid::now_v7();
            let uow = ctx.uow("contract").await?;
            uow.write(key).await?;

            check!(uow.exists(key).await?);
            check!(!ctx.exists(key).await?);
            uow.commit().await?;
            check!(ctx.exists(key).await?);
            Ok(())
        }

        pub async fn rollback_keeps_writes_made_outside<C>(ctx: &C) -> anyhow::Result<()>
        where
            C: HasUnitOfWork<UoW: Probe> + Probe,
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
            C: HasUnitOfWork<UoW: Probe> + Probe,
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

    /// A context whose units of work fail with `fails` at each step.
    struct Failing {
        begin: Option<FailureKind>,
        end: Option<FailureKind>,
    }

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
        type UoW = FailingUow;

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
        let Err(error) = ctx.uow("create group").await else {
            panic!("begin fails");
        };
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

    /// Rows in memory; a unit of work writes through and puts back the rows it started from
    /// unless committed, or every write when `leaks`.
    #[cfg(feature = "testing")]
    #[derive(Default, Clone)]
    struct Memory {
        rows: std::sync::Arc<std::sync::Mutex<std::collections::HashSet<crate::Uuid>>>,
        leaks: bool,
    }

    #[cfg(feature = "testing")]
    struct MemoryUow {
        memory: Memory,
        start: Option<std::collections::HashSet<crate::Uuid>>,
    }

    #[cfg(feature = "testing")]
    impl contract::Probe for Memory {
        async fn write(&self, key: crate::Uuid) -> anyhow::Result<()> {
            self.rows.lock().unwrap().insert(key);
            Ok(())
        }

        async fn exists(&self, key: crate::Uuid) -> anyhow::Result<bool> {
            Ok(self.rows.lock().unwrap().contains(&key))
        }
    }

    #[cfg(feature = "testing")]
    impl contract::Probe for MemoryUow {
        async fn write(&self, key: crate::Uuid) -> anyhow::Result<()> {
            self.memory.write(key).await
        }

        async fn exists(&self, key: crate::Uuid) -> anyhow::Result<bool> {
            self.memory.exists(key).await
        }
    }

    #[cfg(feature = "testing")]
    impl HasUnitOfWork for Memory {
        type UoW = MemoryUow;

        async fn begin(&self) -> Result<MemoryUow, Failure> {
            let start = (!self.leaks).then(|| self.rows.lock().unwrap().clone());
            Ok(MemoryUow {
                memory: self.clone(),
                start,
            })
        }
    }

    #[cfg(feature = "testing")]
    impl UnitOfWork for MemoryUow {
        async fn commit(mut self) -> Result<(), Failure> {
            self.start = None;
            Ok(())
        }

        async fn rollback(self) -> Result<(), Failure> {
            Ok(())
        }
    }

    #[cfg(feature = "testing")]
    impl Drop for MemoryUow {
        fn drop(&mut self) {
            if let Some(start) = self.start.take() {
                *self.memory.rows.lock().unwrap() = start;
            }
        }
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    async fn the_contract_passes_a_unit_of_work_that_rolls_back() {
        contract::run_all(async || Memory::default()).await;
    }

    #[cfg(feature = "testing")]
    #[tokio::test]
    async fn the_contract_fails_a_unit_of_work_that_keeps_its_writes() {
        let ran = tokio::spawn(contract::run_all(async || Memory {
            leaks: true,
            ..Memory::default()
        }))
        .await;

        check!(ran.is_err_and(|error| error.is_panic()));
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
