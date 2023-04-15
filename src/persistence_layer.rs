// SPDX-FileCopyrightText: 2021 Changgyoo Park <wvwwvwwv@me.com>
//
// SPDX-License-Identifier: Apache-2.0

use super::{Database, Error, JournalID, Sequencer, TransactionID};
use std::fmt::Debug;
use std::future::Future;
use std::marker::PhantomData;
use std::pin::Pin;
use std::task::{Context, Poll, Waker};
use std::time::Instant;

/// The [`PersistenceLayer`] trait defines the interface between [`Database`](super::Database) and
/// the persistence layer of the database.
///
/// [`PersistenceLayer`] implementations must be linearizable such that any [`PersistenceLayer`]
/// method invocation after a previously concluded [`PersistenceLayer`] call should be recovered
/// in the same order, e.g., `commit(1, 1)` returned an [`AwaitIO`], and then `commit(2, 2)` is
/// invoked, `commit(1, 1)` should be recovered before `commit(2, 2)` whether or not the returned
/// [`AwaitIO`] was awaited.
pub trait PersistenceLayer<S: Sequencer>: 'static + Debug + Send + Sized + Sync {
    /// Recovers the database.
    ///
    /// If a specific logical instant is specified, it only recovers the storage up until the time
    /// point.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] if the database could not be recovered.
    fn recover(
        &self,
        database: &mut Database<S, Self>,
        until: Option<S::Instant>,
        deadline: Option<Instant>,
    ) -> Result<AwaitIO<S, Self>, Error>;

    /// Backs up the complete database.
    ///
    /// If a path is specified, backed up data is stored in it, otherwise a default path set by the
    /// persistence layer will be used.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] if the database could not be backed up.
    fn backup(
        &self,
        database: &Database<S, Self>,
        catalog_only: bool,
        path: Option<&str>,
        deadline: Option<Instant>,
    ) -> Result<AwaitIO<S, Self>, Error>;

    /// Manually generates a checkpoint.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] if a checkpoint could not be generated within the specified deadline.
    fn checkpoint(
        &self,
        database: &Database<S, Self>,
        deadline: Option<Instant>,
    ) -> Result<AwaitIO<S, Self>, Error>;

    /// The transaction is participating in a distributed transaction.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] if the content of the log record could not be passed to the device.
    fn participate(
        &self,
        id: TransactionID,
        xid: &[u8],
        deadline: Option<Instant>,
    ) -> Result<AwaitIO<S, Self>, Error>;

    /// The database is being modified by the [`Journal`](super::Journal).
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] if the content of the log record could not be passed to the device.
    fn record(
        &self,
        id: TransactionID,
        journal_id: JournalID,
        content: &[u8],
        deadline: Option<Instant>,
    ) -> Result<AwaitIO<S, Self>, Error>;

    /// A [`Journal`](super::Journal) was submitted.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] if the content of the log record could not be passed to the device.
    fn submit(
        &self,
        id: TransactionID,
        journal_id: JournalID,
        transaction_instant: usize,
        deadline: Option<Instant>,
    ) -> Result<AwaitIO<S, Self>, Error>;

    /// A transaction is being rewound.
    ///
    /// Rewinding the transaction to `transaction_instant == 0` amounts to rolling back the entire
    /// transaction.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] if the content of the log record could not be passed to the device.
    fn rewind(
        &self,
        id: TransactionID,
        transaction_instant: usize,
        deadline: Option<Instant>,
    ) -> Result<AwaitIO<S, Self>, Error>;

    /// A transaction is being prepared for commit.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] if the content of the log record could not be passed to the device.
    fn prepare(
        &self,
        id: TransactionID,
        prepare_instant: S::Instant,
        deadline: Option<Instant>,
    ) -> Result<AwaitIO<S, Self>, Error>;

    /// A transaction is being committed.
    ///
    /// It constructs the content of a log record containing the fact that the transaction is being
    /// committed at the specified time point, and then returns a [`Future`] that actually waits
    /// for the content to be persisted.
    ///
    /// # Errors
    ///
    /// Returns an [`Error`] if the content of the log record could not be passed to the device.
    fn commit(
        &self,
        id: TransactionID,
        commit_instant: S::Instant,
        deadline: Option<Instant>,
    ) -> Result<AwaitIO<S, Self>, Error>;

    /// Checks if the IO operation was completed.
    ///
    /// If the IO operation is still in progress, the supplied [`Waker`] is kept in the
    /// [`PersistenceLayer`] and notifies it when the operation is completed.
    ///
    /// It returns the latest known logical instant value of the database.
    fn check(&self, io_id: usize, waker: &Waker) -> Option<Result<S::Instant, Error>>;
}

/// [`AwaitIO`] is returned by a [`PersistenceLayer`] if the content of a log record was
/// successfully materialized in memory and ready for being persisted.
///
/// Dropping an [`AwaitIO`] without awaiting it is allowed if the user does not need to wait for an
/// IO completion. If an [`AwaitIO`] returns an [`Error`], all the future [`PersistenceLayer`]
/// operations shall fail until recovered.
#[derive(Debug)]
pub struct AwaitIO<'p, S: Sequencer, P: PersistenceLayer<S>> {
    /// The persistence layer by which the IO operation is performed.
    persistence_layer: &'p P,

    /// The IO operation identifier.
    io_id: usize,

    /// The deadline of the IO operation.
    deadline: Option<Instant>,

    /// Phantom to use `S`.
    _phantom: PhantomData<S>,
}

/// Volatile memory device.
///
/// The content of the device is not persisted at all.
#[derive(Debug, Default)]
pub struct MemoryDevice<S: Sequencer> {
    _phantom: std::marker::PhantomData<S>,
}

impl<'p, S: Sequencer, P: PersistenceLayer<S>> AwaitIO<'p, S, P> {
    /// Forgets the IO operation.
    #[inline]
    pub fn forget(self) {
        // Do nothing.
    }
}

impl<'p, S: Sequencer, P: PersistenceLayer<S>> Future for AwaitIO<'p, S, P> {
    type Output = Result<S::Instant, Error>;

    #[inline]
    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        if let Some(result) = self.persistence_layer.check(self.io_id, cx.waker()) {
            Poll::Ready(result)
        } else if self
            .deadline
            .as_ref()
            .map_or(false, |d| *d < Instant::now())
        {
            Poll::Ready(Err(Error::Timeout))
        } else {
            // It assumes that the persistence layer will wake up the executor when ready.
            Poll::Pending
        }
    }
}

impl<S: Sequencer> PersistenceLayer<S> for MemoryDevice<S> {
    #[inline]
    fn recover(
        &self,
        _database: &mut Database<S, Self>,
        _until: Option<<S as Sequencer>::Instant>,
        deadline: Option<Instant>,
    ) -> Result<AwaitIO<S, Self>, Error> {
        Ok(AwaitIO {
            persistence_layer: self,
            io_id: 0,
            deadline,
            _phantom: PhantomData,
        })
    }

    #[inline]
    fn backup(
        &self,
        _database: &Database<S, Self>,
        _catalog_only: bool,
        _path: Option<&str>,
        deadline: Option<Instant>,
    ) -> Result<AwaitIO<S, Self>, Error> {
        Ok(AwaitIO {
            persistence_layer: self,
            io_id: 0,
            deadline,
            _phantom: PhantomData,
        })
    }

    #[inline]
    fn checkpoint(
        &self,
        _database: &Database<S, Self>,
        deadline: Option<Instant>,
    ) -> Result<AwaitIO<S, Self>, Error> {
        Ok(AwaitIO {
            persistence_layer: self,
            io_id: 0,
            deadline,
            _phantom: PhantomData,
        })
    }

    #[inline]
    fn participate(
        &self,
        _id: TransactionID,
        _xid: &[u8],
        deadline: Option<Instant>,
    ) -> Result<AwaitIO<S, Self>, Error> {
        Ok(AwaitIO {
            persistence_layer: self,
            io_id: 0,
            deadline,
            _phantom: PhantomData,
        })
    }

    #[inline]
    fn record(
        &self,
        _id: TransactionID,
        _journal_id: JournalID,
        _content: &[u8],
        deadline: Option<Instant>,
    ) -> Result<AwaitIO<S, Self>, Error> {
        Ok(AwaitIO {
            persistence_layer: self,
            io_id: 0,
            deadline,
            _phantom: PhantomData,
        })
    }

    #[inline]
    fn submit(
        &self,
        _id: TransactionID,
        _journal_id: JournalID,
        _transaction_instant: usize,
        deadline: Option<Instant>,
    ) -> Result<AwaitIO<S, Self>, Error> {
        Ok(AwaitIO {
            persistence_layer: self,
            io_id: 0,
            deadline,
            _phantom: PhantomData,
        })
    }

    #[inline]
    fn rewind(
        &self,
        _id: TransactionID,
        _transaction_instant: usize,
        deadline: Option<Instant>,
    ) -> Result<AwaitIO<S, Self>, Error> {
        Ok(AwaitIO {
            persistence_layer: self,
            io_id: 0,
            deadline,
            _phantom: PhantomData,
        })
    }

    #[inline]
    fn prepare(
        &self,
        _id: TransactionID,
        _prepare_instant: <S as Sequencer>::Instant,
        deadline: Option<Instant>,
    ) -> Result<AwaitIO<S, Self>, Error> {
        Ok(AwaitIO {
            persistence_layer: self,
            io_id: 0,
            deadline,
            _phantom: PhantomData,
        })
    }

    #[inline]
    fn commit(
        &self,
        _id: TransactionID,
        _commit_instant: <S as Sequencer>::Instant,
        deadline: Option<Instant>,
    ) -> Result<AwaitIO<S, Self>, Error> {
        Ok(AwaitIO {
            persistence_layer: self,
            io_id: 0,
            deadline,
            _phantom: PhantomData,
        })
    }

    #[inline]
    fn check(&self, _io_id: usize, _waker: &Waker) -> Option<Result<S::Instant, Error>> {
        Some(Ok(S::Instant::default()))
    }
}
