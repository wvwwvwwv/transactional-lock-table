<!--
SPDX-FileCopyrightText: 2021 Changgyoo Park <wvwwvwwv@me.com>

SPDX-License-Identifier: Apache-2.0
-->

# Transactional Lock Table

![GitHub Workflow Status](https://img.shields.io/github/actions/workflow/status/wvwwvwwv/transactional-lock-table/tlt.yml?branch=main)

* WORK_IN_PROGRESS

### Examples

```rust
use tlt::LockTable;

async {
    let lock_table = LockTable::default();

    let transaction = lock_table.transaction();
    let mut sub_transaction = transaction.sub_transaction();

    // WIP
};
 ```

## [Changelog](https://github.com/SAP/transactional-lock-table/blob/main/CHANGELOG.md)
