use super::transaction::Anchor as TransactionAnchor;
use super::transaction::RecordData;
use super::{Error, Sequencer, Snapshot, Transaction, Version};

use std::sync::{Condvar, Mutex};

use scc::ebr;

/// [Journal] keeps the change history.
///
/// Locks and log records are accumulated in a [Journal].
pub struct Journal<'s, 't, S: Sequencer> {
    transaction: &'t Transaction<'s, S>,
    records: RecordData<S>,
}

impl<'s, 't, S: Sequencer> Journal<'s, 't, S> {
    /// Submits the [Journal], thereby advancing the logical clock of the corresponding
    /// [Transaction], making its changes possible to be committed to the database.
    ///
    /// It returns the updated transaction-local clock value.
    ///
    /// # Examples
    ///
    /// ```
    /// use tss::{AtomicCounter, Log, Storage, Transaction};
    ///
    /// let storage: Storage<AtomicCounter> = Storage::new(None);
    /// let transaction = storage.transaction();
    /// let journal = transaction.start();
    /// assert_eq!(journal.submit(), 1);
    /// ```
    pub fn submit(self) -> usize {
        self.transaction.record(self.records)
    }

    /// Takes a snapshot including changes in the Journal.
    ///
    /// # Examples
    /// ```
    /// use tss::{AtomicCounter, RecordVersion, Storage, Transaction, Version};
    ///
    /// let versioned_object = RecordVersion::new();
    /// let storage: Storage<AtomicCounter> = Storage::new(None);
    /// let mut transaction = storage.transaction();
    ///
    /// let mut journal = transaction.start();
    /// assert!(journal.create(&versioned_object, None).is_ok());
    ///
    /// let snapshot = journal.snapshot();
    /// drop(snapshot);
    /// ```
    pub fn snapshot<'r>(&'r self) -> Snapshot<'s, 't, 'r, S> {
        Snapshot::new(
            self.transaction.sequencer(),
            Some(self.transaction),
            Some(self),
        )
    }

    /// Creates a versioned database object.
    ///
    /// The acquired lock is never released until the [Journal] is dropped. If the lock is
    /// released without a valid clock value assigned to the [Journal], the version is either
    /// be properly initialized by another [Journal], or garbage-collected later.
    ///
    /// # Examples
    ///
    /// ```
    /// use tss::{AtomicCounter, RecordVersion, Storage, Transaction, Version};
    ///
    /// let versioned_object = RecordVersion::new();
    /// let storage: Storage<AtomicCounter> = Storage::new(None);
    /// let mut transaction = storage.transaction();
    ///
    /// let mut journal = transaction.start();
    /// assert!(journal.create(&versioned_object, None).is_ok());
    /// journal.submit();
    ///
    /// transaction.commit();
    ///
    /// let snapshot = storage.snapshot();
    /// let guard = crossbeam_epoch::pin();
    /// assert!(versioned_object.predate(&snapshot, &guard));
    /// ```
    pub fn create<V: Version<S>>(
        &mut self,
        version: &V,
        payload: Option<V::Data>,
    ) -> Result<(), Error> {
        let barrier = ebr::Barrier::new();
        let version_cell_ptr = version.version_cell_ptr(&barrier);
        if let Some(version_ref) = version_cell_ptr.as_ref() {
            if let Some(locker) = version.create(self.records.anchor(&barrier), &barrier) {
                self.records.record(version, locker, payload, &barrier);
                return Ok(());
            }
        }

        // The versioned object is not ready for versioning.
        Err(Error::Fail)
    }

    /// Creates a new [Journal].
    pub(super) fn new(
        transaction: &'t Transaction<'s, S>,
        records: RecordData<S>,
    ) -> Journal<'s, 't, S> {
        Journal {
            transaction,
            records,
        }
    }
}

/// [Anchor] is a piece of data that outlives its associated [Journal].
///
/// [VersionCell](super::version::VersionCell) may point to it if the [Journal] owns the
/// [Version].
pub(super) struct Anchor<S: Sequencer> {
    transaction_anchor: ebr::Arc<TransactionAnchor<S>>,
    wait_queue: (Mutex<(bool, usize)>, Condvar),
    creation_clock: usize,
    submit_clock: usize,
    _pin: std::marker::PhantomPinned,
}

impl<S: Sequencer> Anchor<S> {
    pub(super) fn new(
        transaction_anchor: ebr::Arc<TransactionAnchor<S>>,
        creation_clock: usize,
    ) -> Anchor<S> {
        Anchor {
            transaction_anchor,
            wait_queue: (Mutex::new((false, 0)), Condvar::new()),
            creation_clock,
            submit_clock: usize::MAX,
            _pin: std::marker::PhantomPinned,
        }
    }

    pub(super) fn final_snapshot(&self) -> S::Clock {
        self.transaction_anchor.snapshot()
    }

    /// Checks if the lock it has acquired can be transferred to the Journal associated with the given JournalAnchor.
    ///
    /// It returns (true, true) if the given record has started after its data was submitted to the transaction.
    pub fn lockable(&self, journal_anchor: &Anchor<S>, barrier: &ebr::Barrier) -> (bool, bool) {
        if self.transaction_anchor.load(Relaxed, guard)
            != journal_anchor.transaction_anchor.load(Relaxed, guard)
        {
            // Different transactions.
            return (false, false);
        }
        (true, self.submit_clock <= journal_anchor.creation_clock)
    }

    /// Checks whether the transaction clock or record anchor predates self.
    pub fn predate(
        &self,
        transaction: &Transaction<S>,
        transaction_clock: usize,
        journal: Option<&Journal<S>>,
        guard: &Guard,
    ) -> bool {
        if self.transaction_anchor.load(Relaxed, guard)
            != transaction.anchor_ptr.load(Relaxed, guard)
        {
            // Different transactions.
            return false;
        }
        let submit_clock = self.submit_clock;
        if submit_clock != usize::MAX && submit_clock <= transaction_clock {
            // It was submitted and predates the given transaction local clock.
            return true;
        }
        // The given anchor is itself.
        journal.map_or_else(
            || false,
            |journal| journal.records.anchor_ptr.load(Relaxed, guard).as_raw() == self as *const _,
        )
    }

    /// The transaction record has either been committed or rolled back.
    fn end(&self) {
        if let Ok(mut wait_queue) = self.wait_queue.0.lock() {
            if !wait_queue.0 {
                // Setting the flag true has an immediate effect on all the versioned owned by the RecordData.
                //  - It allows all the other transaction to have a chance to take ownership of the versioned objects.
                wait_queue.0 = true;
                self.wait_queue.1.notify_one();
            }
        }

        // Asynchronously post-processes with the mutex acquired.
        //
        // Still, the RecordData is holding all the VersionLock instances.
        // therefore, it firstly wakes all the waiting threads up before releasing the locks.
        while let Ok(wait_queue) = self.wait_queue.0.lock() {
            if wait_queue.1 == 0 {
                break;
            }
            drop(wait_queue);
        }
    }

    /// Returns the submit-time clock value.
    pub fn submit_clock(&self) -> usize {
        self.submit_clock
    }

    /// Waits for the final state of the RecordData to be determined.
    pub fn wait<R, F: FnOnce(&S::Clock) -> R>(&self, f: F, guard: &Guard) -> Option<R> {
        if let Ok(mut wait_queue) = self.wait_queue.0.lock() {
            while !wait_queue.0 {
                wait_queue.1 += 1;
                wait_queue = self.wait_queue.1.wait(wait_queue).unwrap();
                wait_queue.1 -= 1;
            }
            // Before waking up the next waiting thread, call the given function with the mutex acquired.
            //  - For instance, if the version is owned by the transaction, ownership can be transferred.
            let result = f(unsafe {
                &self
                    .transaction_anchor
                    .load(Acquire, guard)
                    .deref()
                    .snapshot()
            });

            // Once the thread wakes up, it is mandated to wake the next thread up.
            if wait_queue.1 > 0 {
                self.wait_queue.1.notify_one();
            }

            return Some(result);
        }
        None
    }

    /// Returns true if the transaction is visible to the reader.
    pub fn visible(&self, snapshot: &S::Clock, barrier: &ebr::Barrier) -> (bool, S::Clock) {
        let anchor_ref = unsafe { self.transaction_anchor.load(Acquire, barrier).deref() };
        if anchor_ref.preliminary_snapshot == S::Clock::default()
            || anchor_ref.preliminary_snapshot >= *snapshot
        {
            return (false, S::Clock::default());
        }
        // The transaction will either be committed or rolled back soon.
        if anchor_ref.final_snapshot == S::Clock::default() {
            self.wait(|_| (), barrier);
        }
        // Checks the final snapshot.
        let final_snapshot = anchor_ref.final_snapshot;
        (
            final_snapshot != S::Clock::default() && final_snapshot <= *snapshot,
            final_snapshot,
        )
    }
}
