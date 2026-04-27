use anyhow::Context as _;
use serde::Serialize;
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

pub use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Result as RusqliteResult, Row, params,
};

pub const SQLITE_BUSY_TIMEOUT_MS: u64 = 5_000;
const SQLITE_LOCK_RETRY_SLEEP_MS: u64 = 50;

#[derive(Debug, Clone, Copy)]
pub struct SqliteConnectionOptions {
    pub db_name: &'static str,
    pub operation: &'static str,
    pub busy_timeout_ms: u64,
    pub journal_mode: Option<&'static str>,
    pub synchronous: Option<&'static str>,
    pub foreign_keys: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct SqliteLockTelemetryEntry {
    pub db: String,
    pub operation: String,
    pub wait_stage: String,
    pub busy_count: u64,
    pub lock_wait_ms: u64,
    pub flock_acquire_latency_ms: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
struct SqliteLockTelemetryKey {
    db: String,
    operation: String,
    wait_stage: String,
}

#[derive(Debug, Default)]
struct SqliteLockTelemetryCounters {
    busy_count: AtomicU64,
    lock_wait_ms: AtomicU64,
    flock_acquire_latency_ms: AtomicU64,
}

static SQLITE_LOCK_TELEMETRY: OnceLock<
    Mutex<BTreeMap<SqliteLockTelemetryKey, Arc<SqliteLockTelemetryCounters>>>,
> = OnceLock::new();

#[must_use]
pub fn sqlite_error_is_locked(error: &rusqlite::Error) -> bool {
    match error {
        rusqlite::Error::SqliteFailure(code, message) => {
            matches!(
                code.code,
                rusqlite::ErrorCode::DatabaseBusy | rusqlite::ErrorCode::DatabaseLocked
            ) || message.as_deref().is_some_and(|value| {
                value.contains("database is locked") || value.contains("database table is locked")
            })
        }
        _ => false,
    }
}

/// Apply the shared worldsim `SQLite` configuration surface to an open connection.
///
/// # Errors
/// Returns an error when `busy_timeout` or any requested PRAGMA update fails.
pub fn configure_sqlite_connection(
    conn: &Connection,
    options: SqliteConnectionOptions,
) -> anyhow::Result<()> {
    conn.busy_timeout(Duration::from_millis(options.busy_timeout_ms))
        .with_context(|| format!("{} busy_timeout failed", options.db_name))?;

    if options.foreign_keys {
        conn.execute_batch("PRAGMA foreign_keys = ON;")
            .with_context(|| format!("{} foreign_keys failed", options.db_name))?;
    }
    if let Some(journal_mode) = options.journal_mode
        && !pragma_already_matches(conn, "journal_mode", journal_mode)?
    {
        apply_pragma_with_retry(conn, options, "journal_mode", journal_mode)?;
    }
    if let Some(synchronous) = options.synchronous
        && !pragma_already_matches(conn, "synchronous", synchronous)?
    {
        apply_pragma_with_retry(conn, options, "synchronous", synchronous)?;
    }

    Ok(())
}

pub fn record_sqlite_busy_wait(db: &str, operation: &str, wait_stage: &str, wait_ms: u64) {
    let counters = sqlite_lock_telemetry_counters(db, operation, wait_stage);
    counters.busy_count.fetch_add(1, Ordering::Relaxed);
    counters.lock_wait_ms.fetch_add(wait_ms, Ordering::Relaxed);
}

pub fn record_flock_acquire_latency(db: &str, operation: &str, wait_stage: &str, latency_ms: u64) {
    let counters = sqlite_lock_telemetry_counters(db, operation, wait_stage);
    counters
        .flock_acquire_latency_ms
        .fetch_add(latency_ms, Ordering::Relaxed);
}

pub fn sqlite_lock_telemetry_snapshot() -> Vec<SqliteLockTelemetryEntry> {
    sqlite_lock_telemetry_registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .iter()
        .map(|(key, counters)| SqliteLockTelemetryEntry {
            db: key.db.clone(),
            operation: key.operation.clone(),
            wait_stage: key.wait_stage.clone(),
            busy_count: counters.busy_count.load(Ordering::Relaxed),
            lock_wait_ms: counters.lock_wait_ms.load(Ordering::Relaxed),
            flock_acquire_latency_ms: counters.flock_acquire_latency_ms.load(Ordering::Relaxed),
        })
        .collect()
}

#[doc(hidden)]
pub fn reset_sqlite_lock_telemetry_for_tests() {
    sqlite_lock_telemetry_registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clear();
}

fn pragma_already_matches(
    conn: &Connection,
    pragma: &str,
    expected: &'static str,
) -> anyhow::Result<bool> {
    match pragma {
        "journal_mode" => {
            let current: String = conn
                .pragma_query_value(None, pragma, |row| row.get(0))
                .with_context(|| format!("query {pragma} failed"))?;
            Ok(current.eq_ignore_ascii_case(expected))
        }
        "synchronous" => {
            let current: i64 = conn
                .pragma_query_value(None, pragma, |row| row.get(0))
                .with_context(|| format!("query {pragma} failed"))?;
            let expected = match expected.to_ascii_uppercase().as_str() {
                "OFF" => 0,
                "NORMAL" => 1,
                "FULL" => 2,
                "EXTRA" => 3,
                other => anyhow::bail!("unsupported synchronous pragma value: {other}"),
            };
            Ok(current == expected)
        }
        other => anyhow::bail!("unsupported pragma match check: {other}"),
    }
}

fn apply_pragma_with_retry(
    conn: &Connection,
    options: SqliteConnectionOptions,
    pragma: &str,
    value: &'static str,
) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_millis(options.busy_timeout_ms);
    let wait_stage = format!("pragma:{pragma}");

    loop {
        match conn.pragma_update(None, pragma, value) {
            Ok(()) => return Ok(()),
            Err(error) if sqlite_error_is_locked(&error) && Instant::now() < deadline => {
                record_sqlite_busy_wait(
                    options.db_name,
                    options.operation,
                    wait_stage.as_str(),
                    SQLITE_LOCK_RETRY_SLEEP_MS,
                );
                std::thread::sleep(Duration::from_millis(SQLITE_LOCK_RETRY_SLEEP_MS));
            }
            Err(error) => {
                return Err(error).with_context(|| format!("{} {pragma} failed", options.db_name));
            }
        }
    }
}

fn sqlite_lock_telemetry_registry()
-> &'static Mutex<BTreeMap<SqliteLockTelemetryKey, Arc<SqliteLockTelemetryCounters>>> {
    SQLITE_LOCK_TELEMETRY.get_or_init(|| Mutex::new(BTreeMap::new()))
}

fn sqlite_lock_telemetry_counters(
    db: &str,
    operation: &str,
    wait_stage: &str,
) -> Arc<SqliteLockTelemetryCounters> {
    let key = SqliteLockTelemetryKey {
        db: db.to_string(),
        operation: operation.to_string(),
        wait_stage: wait_stage.to_string(),
    };
    let mut registry = sqlite_lock_telemetry_registry()
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    registry
        .entry(key)
        .or_insert_with(|| Arc::new(SqliteLockTelemetryCounters::default()))
        .clone()
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn configure_sqlite_connection_applies_pragmas() {
        reset_sqlite_lock_telemetry_for_tests();

        let tempdir = tempfile::tempdir().unwrap();
        let db_path = tempdir.path().join("telemetry.sqlite3");
        let conn = Connection::open(&db_path).unwrap();
        configure_sqlite_connection(
            &conn,
            SqliteConnectionOptions {
                db_name: "test",
                operation: "configure",
                busy_timeout_ms: SQLITE_BUSY_TIMEOUT_MS,
                journal_mode: Some("WAL"),
                synchronous: Some("NORMAL"),
                foreign_keys: true,
            },
        )
        .unwrap();

        let journal_mode: String = conn
            .pragma_query_value(None, "journal_mode", |row| row.get(0))
            .unwrap();
        let synchronous: i64 = conn
            .pragma_query_value(None, "synchronous", |row| row.get(0))
            .unwrap();
        let foreign_keys: i64 = conn
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();

        assert_eq!(journal_mode.to_ascii_lowercase(), "wal");
        assert_eq!(synchronous, 1);
        assert_eq!(foreign_keys, 1);
    }

    #[test]
    fn configure_sqlite_connection_skips_persistent_pragma_rewrites_when_already_set() {
        reset_sqlite_lock_telemetry_for_tests();

        let tempdir = tempfile::tempdir().unwrap();
        let db_path = tempdir.path().join("busy.sqlite3");
        let bootstrap = Connection::open(&db_path).unwrap();
        configure_sqlite_connection(
            &bootstrap,
            SqliteConnectionOptions {
                db_name: "test",
                operation: "bootstrap",
                busy_timeout_ms: 50,
                journal_mode: Some("WAL"),
                synchronous: Some("NORMAL"),
                foreign_keys: false,
            },
        )
        .unwrap();
        bootstrap
            .execute_batch(
                "
                CREATE TABLE IF NOT EXISTS busy_probe (id INTEGER PRIMARY KEY);
                INSERT INTO busy_probe(id) VALUES (1);
                ",
            )
            .unwrap();
        drop(bootstrap);

        let blocker = Connection::open(&db_path).unwrap();
        blocker
            .execute_batch(
                "
                BEGIN IMMEDIATE TRANSACTION;
                UPDATE busy_probe SET id = id WHERE id = 1;
                ",
            )
            .unwrap();

        let reopened = Connection::open(&db_path).unwrap();
        configure_sqlite_connection(
            &reopened,
            SqliteConnectionOptions {
                db_name: "test",
                operation: "reopen_current",
                busy_timeout_ms: 50,
                journal_mode: Some("WAL"),
                synchronous: Some("NORMAL"),
                foreign_keys: false,
            },
        )
        .unwrap();

        blocker.execute_batch("ROLLBACK;").unwrap();
    }

    #[test]
    fn sqlite_lock_telemetry_snapshot_groups_by_dimension() {
        reset_sqlite_lock_telemetry_for_tests();

        record_sqlite_busy_wait("world_db", "write_core", "pragma:journal_mode", 50);
        record_sqlite_busy_wait("world_db", "write_core", "pragma:journal_mode", 25);
        record_flock_acquire_latency("world_db", "write_core", "flock_acquire", 7);

        let snapshot = sqlite_lock_telemetry_snapshot();
        assert_eq!(snapshot.len(), 2);

        let busy_entry = snapshot
            .iter()
            .find(|entry| entry.wait_stage == "pragma:journal_mode")
            .unwrap();
        assert_eq!(busy_entry.busy_count, 2);
        assert_eq!(busy_entry.lock_wait_ms, 75);
        assert_eq!(busy_entry.flock_acquire_latency_ms, 0);

        let flock_entry = snapshot
            .iter()
            .find(|entry| entry.wait_stage == "flock_acquire")
            .unwrap();
        assert_eq!(flock_entry.busy_count, 0);
        assert_eq!(flock_entry.lock_wait_ms, 0);
        assert_eq!(flock_entry.flock_acquire_latency_ms, 7);
    }
}
