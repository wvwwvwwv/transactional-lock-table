// SPDX-FileCopyrightText: 2021 Changgyoo Park <wvwwvwwv@me.com>
//
// SPDX-License-Identifier: Apache-2.0

#![deny(
    missing_docs,
    warnings,
    clippy::all,
    clippy::pedantic,
    clippy::undocumented_unsafe_blocks
)]

//! Transactional Lock Table

mod lock_table;
pub use lock_table::AccessController;

mod error;
pub use error::Error;

mod accessor;
pub use accessor::Journal;

mod transaction;
pub use transaction::{Committable, Transaction};

pub mod utils;

#[cfg(test)]
mod tests;
