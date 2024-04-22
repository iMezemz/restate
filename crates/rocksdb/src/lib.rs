// Copyright (c) 2024 -  Restate Software, Inc., Restate GmbH.
// All rights reserved.
//
// Use of this software is governed by the Business Source License
// included in the LICENSE file.
//
// As of the Change Date specified in that file, in accordance with
// the Business Source License, use of this software will be governed
// by the Apache License, Version 2.0.

mod db_manager;
mod db_spec;
mod error;
mod rock_access;

pub use db_manager::RocksDbManager;
pub use db_spec::*;
pub use error::*;
use restate_core::ShutdownError;
use restate_types::config::RocksDbOptions;
pub use rock_access::*;
use tracing::debug;
use tracing::warn;

use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;

use rocksdb::statistics::Histogram;
use rocksdb::statistics::HistogramData;
use rocksdb::statistics::Ticker;

type BoxedCfMatcher = Box<dyn CfNameMatch + Send + Sync>;
type BoxedCfOptionUpdater = Box<dyn Fn(rocksdb::Options) -> rocksdb::Options + Send + Sync>;

#[derive(derive_more::Display, Clone)]
#[display(fmt = "{}::{}", owner, name)]
pub struct RocksDb {
    manager: &'static RocksDbManager,
    pub owner: Owner,
    pub name: DbName,
    pub path: PathBuf,
    pub db_options: rocksdb::Options,
    cf_patterns: Arc<[(BoxedCfMatcher, BoxedCfOptionUpdater)]>,
    flush_on_shutdown: Arc<[BoxedCfMatcher]>,
    db: Arc<dyn RocksAccess + Send + Sync + 'static>,
}

static_assertions::assert_impl_all!(RocksDb: Send, Sync);

impl Deref for RocksDb {
    type Target = Arc<dyn RocksAccess + Send + Sync + 'static>;

    fn deref(&self) -> &Self::Target {
        &self.db
    }
}

impl RocksDb {
    pub(crate) fn new<T>(manager: &'static RocksDbManager, spec: DbSpec<T>, db: Arc<T>) -> Self
    where
        T: RocksAccess + Send + Sync + 'static,
    {
        Self {
            manager,
            owner: spec.owner,
            name: spec.name,
            path: spec.path,
            cf_patterns: spec.cf_patterns.into(),
            db,
            db_options: spec.db_options,
            flush_on_shutdown: spec.flush_on_shutdown.into(),
        }
    }

    /// Returns the raw rocksdb handle, this should only be used for server operations that
    /// require direct access to rocksdb.
    pub fn inner(&self) -> &Arc<dyn RocksAccess + Send + Sync + 'static> {
        &self.db
    }

    pub fn cfs(&self) -> Vec<CfName> {
        self.db.cfs()
    }

    pub fn get_histogram_data(&self, histogram: Histogram) -> HistogramData {
        self.db_options.get_histogram_data(histogram)
    }

    pub fn get_ticker_count(&self, ticker: Ticker) -> u64 {
        self.db_options.get_ticker_count(ticker)
    }

    pub fn get_statistics_str(&self) -> Option<String> {
        self.db_options.get_statistics()
    }

    pub async fn open_cf(&self, name: CfName, opts: &RocksDbOptions) -> Result<(), RocksError> {
        let default_cf_options = self.manager.default_cf_options(opts);
        // todo Run in the background storage thread pool
        let db = self.db.clone();
        let cf_patterns = self.cf_patterns.clone();
        tokio::task::spawn_blocking(move || db.open_cf(name, default_cf_options, cf_patterns))
            .await
            .map_err(|_| RocksError::Shutdown(ShutdownError))?
    }

    pub async fn shutdown(&self) {
        if let Err(e) = self.flush_wal(true) {
            warn!(
                db = %self.name,
                owner = %self.owner,
                "Failed to flush local loglet rocksdb WAL: {}",
                e
            );
        }

        let cfs_to_flush = self
            .cfs()
            .into_iter()
            .filter(|c| {
                self.flush_on_shutdown
                    .iter()
                    .any(|matcher| matcher.cf_matches(c))
            })
            .collect::<Vec<_>>();
        if cfs_to_flush.is_empty() {
            debug!(
                db = %self.name,
                owner = %self.owner,
                "No column families to flush for db on shutdown"
            );
            return;
        }

        debug!(
            db = %self.name,
            owner = %self.owner,
            "Numbre of column families to flush on shutdown: {}", cfs_to_flush.len());
        if let Err(e) = self.db.flush_memtables(cfs_to_flush.as_slice(), true) {
            warn!(
                db = %self.name,
                owner = %self.owner,
                "Failed to flush memtables: {}",
                e
            );
        }
        self.db.cancel_all_background_work(true);
    }
}
