// SPDX-FileCopyrightText: 2021 Changgyoo Park <wvwwvwwv@me.com>
//
// SPDX-License-Identifier: Apache-2.0

/// [`Error`] defines all the error codes used in the lock table implementation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum Error {
    /// The operation conflicts with others.
    Conflict,

    /// The operation causes a deadlock.
    Deadlock,

    /// Memory allocation failed.
    OutOfMemory,

    /// The operation was timed out.
    Timeout,
}
