//! Optional SQLite persistence (feature `storage`).
//!
//! Events are stored with their raw body retained, so unknown event types are
//! never lost and can be decoded later. A per-device sync cursor enables
//! incremental syncs. Re-syncing is idempotent: identical events are de-duped.

use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use rusqlite::{params, Connection, OptionalExtension};

use crate::error::{Error, Result};
use oura_protocol::device::{Battery, DeviceInfo};
use oura_protocol::events::RingEvent;

/// The schema this build writes (`PRAGMA user_version`). Bump it together with a
/// migration step in [`Store::migrate`] whenever a table/index changes: the DB
/// now ships inside the iOS app, so older files must upgrade in place and newer
/// files must be refused rather than silently misread.
pub const SCHEMA_VERSION: i64 = 3;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS device (
    serial        TEXT PRIMARY KEY,
    hardware_id   TEXT,
    firmware      TEXT,
    api_version   TEXT,
    mac           TEXT,
    updated_unix  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS sync_state (
    serial        TEXT PRIMARY KEY,
    next_cursor   INTEGER NOT NULL,
    last_sync_unix INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS events (
    id             INTEGER PRIMARY KEY AUTOINCREMENT,
    serial         TEXT NOT NULL,
    tag            INTEGER NOT NULL,
    name           TEXT NOT NULL,
    ring_timestamp INTEGER NOT NULL,
    body           BLOB NOT NULL,
    decoded_json   TEXT,
    captured_unix  INTEGER NOT NULL,
    UNIQUE(serial, tag, ring_timestamp, body)
);
CREATE INDEX IF NOT EXISTS idx_events_serial_tag ON events(serial, tag);
CREATE INDEX IF NOT EXISTS idx_events_capture ON events(captured_unix, id);
CREATE INDEX IF NOT EXISTS idx_events_tag_time ON events(tag, ring_timestamp);

CREATE TABLE IF NOT EXISTS readings (
    id            INTEGER PRIMARY KEY AUTOINCREMENT,
    serial        TEXT NOT NULL,
    kind          TEXT NOT NULL,
    value         REAL NOT NULL,
    unit          TEXT,
    captured_unix INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_readings_serial_kind ON readings(serial, kind);
"#;

/// v3: one value per `(serial, day, metric)`. `day` is the local date
/// (`YYYY-MM-DD`) the value belongs to. The table is a cache of results computed
/// from `events`; a client can drop every row and compute them again.
const DAILY_SUMMARY_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS daily_summary (
    serial       TEXT NOT NULL,
    day          TEXT NOT NULL,
    metric       TEXT NOT NULL,
    value        REAL NOT NULL,
    updated_unix INTEGER NOT NULL,
    PRIMARY KEY (serial, day, metric)
) WITHOUT ROWID;
"#;

fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// A SQLite-backed store for ring data.
pub struct Store {
    pub(crate) conn: Connection,
}

impl Store {
    /// Open (creating if needed) a database at `path` and ensure the schema.
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path = path.as_ref();
        let conn = Connection::open(path)?;
        // Health data + device identifiers are sensitive; keep the DB owner-only.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
                .map_err(|e| crate::error::Error::Storage(e.to_string()))?;
        }
        conn.busy_timeout(std::time::Duration::from_millis(5000))?;
        let mode: String = conn.query_row("PRAGMA journal_mode=WAL", [], |r| r.get(0))?;
        if mode != "wal" {
            return Err(crate::error::Error::Storage(format!(
                "WAL unavailable: {mode}"
            )));
        }
        conn.execute_batch("PRAGMA synchronous=FULL;")?;
        conn.execute_batch(SCHEMA)?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    /// Read without changing schema, permissions, or journal mode (including bundled seeds).
    pub fn open_read_only<P: AsRef<Path>>(path: P) -> Result<Self> {
        let conn = Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        conn.busy_timeout(std::time::Duration::from_millis(5000))?;
        Ok(Self { conn })
    }

    /// Commit a complete protocol batch and its cursor together, before ACK/progress.
    pub fn commit_batch(&self, serial: &str, events: &[RingEvent], cursor: u32) -> Result<u32> {
        let tx = self.conn.unchecked_transaction()?;
        let mut inserted = 0;
        for event in events {
            inserted += u32::from(self.insert_event(serial, event)?);
        }
        self.set_cursor(serial, cursor)?;
        tx.commit()?;
        Ok(inserted)
    }

    pub fn integrity_check(&self) -> Result<String> {
        Ok(self
            .conn
            .query_row("PRAGMA quick_check", [], |r| r.get(0))?)
    }

    /// Open an in-memory database (useful for tests).
    pub fn open_in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch(SCHEMA)?;
        let store = Self { conn };
        store.migrate()?;
        Ok(store)
    }

    /// Bring an older file up to [`SCHEMA_VERSION`]. Version 0 is both a fresh
    /// file and every pre-versioning file (their tables are identical to v1).
    /// Writing the version is tolerated to fail on a read-only seed DB, like the
    /// WAL switch above.
    fn migrate(&self) -> Result<()> {
        let found = self.schema_version()?;
        if found > SCHEMA_VERSION {
            return Err(Error::SchemaTooNew {
                found,
                supported: SCHEMA_VERSION,
            });
        }
        if found < 2 {
            // v2: indexes for the ring clock (capture order) and time-range reads.
            self.conn.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_events_serial_captured
                     ON events(serial, captured_unix, id);
                 CREATE INDEX IF NOT EXISTS idx_events_serial_ts
                     ON events(serial, ring_timestamp);",
            )?;
        }
        if found < 3 {
            // v3: the per-day metric cache.
            self.conn.execute_batch(DAILY_SUMMARY_SCHEMA)?;
        }
        if found != SCHEMA_VERSION {
            let _ = self
                .conn
                .execute_batch(&format!("PRAGMA user_version = {SCHEMA_VERSION};"));
        }
        Ok(())
    }

    /// The file's `PRAGMA user_version` (0 for a fresh or pre-versioning file).
    pub fn schema_version(&self) -> Result<i64> {
        Ok(self
            .conn
            .query_row("PRAGMA user_version", [], |r| r.get::<_, i64>(0))?)
    }

    /// Record/refresh device metadata.
    pub fn upsert_device(
        &self,
        serial: &str,
        hardware_id: Option<&str>,
        info: Option<&DeviceInfo>,
    ) -> Result<()> {
        self.conn.execute(
            "INSERT INTO device (serial, hardware_id, firmware, api_version, mac, updated_unix)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(serial) DO UPDATE SET
               hardware_id=COALESCE(excluded.hardware_id, device.hardware_id),
               firmware=COALESCE(excluded.firmware, device.firmware),
               api_version=COALESCE(excluded.api_version, device.api_version),
               mac=COALESCE(excluded.mac, device.mac),
               updated_unix=excluded.updated_unix",
            params![
                serial,
                hardware_id,
                info.map(|i| i.firmware_version.clone()),
                info.map(|i| i.api_version.clone()),
                info.map(|i| i.mac.clone()),
                now_unix(),
            ],
        )?;
        Ok(())
    }

    /// Device identity + last-sync for display: the most-recently-updated device
    /// row joined with its sync state.
    /// Returns `(serial, hardware_id, firmware, api_version, mac, updated_unix, last_sync_unix, next_cursor)`.
    #[allow(clippy::type_complexity)]
    pub fn device_info(
        &self,
    ) -> Result<Option<(String, String, String, String, String, i64, i64, i64)>> {
        let row = self
            .conn
            .query_row(
                "SELECT d.serial, COALESCE(d.hardware_id,''), COALESCE(d.firmware,''),
                        COALESCE(d.api_version,''), COALESCE(d.mac,''), COALESCE(d.updated_unix,0),
                        COALESCE(s.last_sync_unix,0), COALESCE(s.next_cursor,0)
                 FROM device d LEFT JOIN sync_state s ON s.serial = d.serial
                 ORDER BY d.updated_unix DESC LIMIT 1",
                [],
                |r| {
                    Ok((
                        r.get::<_, String>(0)?,
                        r.get::<_, String>(1)?,
                        r.get::<_, String>(2)?,
                        r.get::<_, String>(3)?,
                        r.get::<_, String>(4)?,
                        r.get::<_, i64>(5)?,
                        r.get::<_, i64>(6)?,
                        r.get::<_, i64>(7)?,
                    ))
                },
            )
            .optional()?;
        Ok(row)
    }

    /// The persisted incremental-sync cursor (deciseconds), or 0 if none.
    pub fn cursor(&self, serial: &str) -> Result<u32> {
        let v: Option<i64> = self
            .conn
            .query_row(
                "SELECT next_cursor FROM sync_state WHERE serial = ?1",
                params![serial],
                |r| r.get(0),
            )
            .optional()?;
        Ok(v.unwrap_or(0) as u32)
    }

    /// Persist the next sync cursor.
    pub fn set_cursor(&self, serial: &str, cursor: u32) -> Result<()> {
        self.conn.execute(
            "INSERT INTO sync_state (serial, next_cursor, last_sync_unix)
             VALUES (?1, ?2, ?3)
             ON CONFLICT(serial) DO UPDATE SET
               next_cursor=excluded.next_cursor,
               last_sync_unix=excluded.last_sync_unix",
            params![serial, cursor as i64, now_unix()],
        )?;
        Ok(())
    }

    /// Forget the incremental-sync cursor (next drain starts from zero). Used
    /// after a factory reset / fresh pairing, when the ring clock restarted and a
    /// stale high cursor would make every sync come back empty.
    pub fn reset_cursor(&self, serial: &str) -> Result<()> {
        self.set_cursor(serial, 0)
    }

    /// Insert an event, ignoring exact duplicates. Returns true if a row was added.
    pub fn insert_event(&self, serial: &str, ev: &RingEvent) -> Result<bool> {
        self.insert_event_at(serial, ev, now_unix())
    }

    /// [`Self::insert_event`] with an explicit capture time — for imports and for
    /// tests that need deterministic boot epochs.
    pub fn insert_event_at(&self, serial: &str, ev: &RingEvent, captured_unix: i64) -> Result<bool> {
        let decoded = ev
            .decoded
            .as_ref()
            .map(|v| serde_json::to_string(v).unwrap_or_default());
        let changed = self.conn.execute(
            "INSERT OR IGNORE INTO events
               (serial, tag, name, ring_timestamp, body, decoded_json, captured_unix)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                serial,
                ev.tag as i64,
                ev.name,
                ev.timestamp as i64,
                ev.body,
                decoded,
                captured_unix,
            ],
        )?;
        Ok(changed > 0)
    }

    /// Insert many events in one transaction, each with its capture time, for
    /// imports. Exact duplicates are ignored. Returns the number of rows added.
    pub fn insert_events_at(&self, serial: &str, events: &[(RingEvent, i64)]) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        let mut added = 0;
        for (event, captured_unix) in events {
            added += usize::from(self.insert_event_at(serial, event, *captured_unix)?);
        }
        tx.commit()?;
        Ok(added)
    }

    /// Record a scalar reading (e.g. live HR bpm, SpO2 %, battery %).
    pub fn insert_reading(&self, serial: &str, kind: &str, value: f64, unit: &str) -> Result<()> {
        self.conn.execute(
            "INSERT INTO readings (serial, kind, value, unit, captured_unix)
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![serial, kind, value, unit, now_unix()],
        )?;
        Ok(())
    }

    /// Store a battery read: the charge level and the charge progress the ring
    /// reports with it (0 when the ring is not on its charger).
    pub fn insert_battery(&self, serial: &str, battery: &Battery) -> Result<()> {
        self.insert_reading(serial, "battery_percent", battery.percent as f64, "%")?;
        self.insert_reading(
            serial,
            "battery_charging_progress",
            battery.charging_progress as f64,
            "%",
        )
    }

    /// Readings of one `kind` for `serial` as `(captured_unix, value)`, oldest
    /// first, optionally only those captured after `captured_after`.
    pub fn readings(
        &self,
        serial: &str,
        kind: &str,
        captured_after: Option<i64>,
    ) -> Result<Vec<(i64, f64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT captured_unix, value FROM readings \
             WHERE serial = ?1 AND kind = ?2 AND captured_unix > ?3 \
             ORDER BY captured_unix, id",
        )?;
        let rows = stmt
            .query_map(
                params![serial, kind, captured_after.unwrap_or(i64::MIN)],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Write daily metrics as `(day, metric, value)` rows in one transaction. A
    /// metric that is already stored for that day gets the new value; stored
    /// metrics that are not in `rows` stay as they are. A value that is not finite
    /// is skipped. Returns the number of rows written.
    pub fn upsert_daily(&self, serial: &str, rows: &[(&str, &str, f64)]) -> Result<usize> {
        let tx = self.conn.unchecked_transaction()?;
        let now = now_unix();
        let mut written = 0;
        {
            let mut stmt = tx.prepare(
                "INSERT INTO daily_summary (serial, day, metric, value, updated_unix)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(serial, day, metric) DO UPDATE SET
                   value=excluded.value, updated_unix=excluded.updated_unix",
            )?;
            for (day, metric, value) in rows {
                if value.is_finite() {
                    written += stmt.execute(params![serial, day, metric, value, now])?;
                }
            }
        }
        tx.commit()?;
        Ok(written)
    }

    /// Stored metrics for `serial` as `(day, metric, value)` for the days from
    /// `from_day` to `to_day` (both included, `YYYY-MM-DD`), ordered by day.
    pub fn daily_range(
        &self,
        serial: &str,
        from_day: &str,
        to_day: &str,
    ) -> Result<Vec<(String, String, f64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT day, metric, value FROM daily_summary \
             WHERE serial = ?1 AND day >= ?2 AND day <= ?3 ORDER BY day, metric",
        )?;
        let rows = stmt
            .query_map(params![serial, from_day, to_day], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Delete the stored metrics of `serial`. The next summary writes them again.
    pub fn clear_daily(&self, serial: &str) -> Result<usize> {
        Ok(self
            .conn
            .execute("DELETE FROM daily_summary WHERE serial = ?1", params![serial])?)
    }

    /// Re-decode every stored event body with the current decoders, updating
    /// `decoded_json`. Returns `(rows_with_decode, total_rows)`. Lets new decoders
    /// be applied to events captured before they existed — no re-sync needed.
    pub fn redecode(&self) -> Result<(usize, usize)> {
        let rows: Vec<(i64, i64, Vec<u8>)> = {
            let mut stmt = self.conn.prepare("SELECT id, tag, body FROM events")?;
            let collected = stmt
                .query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
                .collect::<std::result::Result<Vec<_>, _>>()?;
            collected
        };
        let total = rows.len();
        let mut decoded_count = 0;
        for (id, tag, body) in rows {
            let decoded = oura_protocol::events::decode_event_body(tag as u8, &body)
                .map(|v| serde_json::to_string(&v).unwrap_or_default());
            if decoded.is_some() {
                decoded_count += 1;
            }
            let name = oura_protocol::events::event_name(tag as u8);
            self.conn.execute(
                "UPDATE events SET decoded_json = ?1, name = ?2 WHERE id = ?3",
                params![decoded, name, id],
            )?;
        }
        Ok((decoded_count, total))
    }

    /// All decoded events as `(ring_timestamp_deciseconds, tag, decoded_json,
    /// captured_unix)`, ordered by ring time. For analysis/reporting commands that
    /// reconstruct time series from stored events.
    pub fn decoded_events(&self) -> Result<Vec<(i64, u8, String, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT ring_timestamp, tag, decoded_json, captured_unix FROM events \
             WHERE decoded_json IS NOT NULL ORDER BY captured_unix, id",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)? as u8,
                    r.get::<_, String>(2)?,
                    r.get::<_, i64>(3)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Every event's `(ring_timestamp, captured_unix, id)` for `serial`, in
    /// capture order — the cheap input for boot-epoch (ring clock) recovery. No
    /// JSON is loaded.
    pub fn event_times(&self, serial: &str) -> Result<Vec<(i64, i64, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT ring_timestamp, captured_unix, id FROM events \
             WHERE serial = ?1 ORDER BY captured_unix, id",
        )?;
        let rows = stmt
            .query_map(params![serial], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Decoded events for `serial` as `(id, ring_timestamp, tag, decoded_json,
    /// captured_unix)` in capture order, optionally restricted to `tags` and to
    /// events captured after `captured_after`.
    #[allow(clippy::type_complexity)]
    pub fn decoded_events_filtered(
        &self,
        serial: &str,
        tags: Option<&[u8]>,
        captured_after: Option<i64>,
    ) -> Result<Vec<(i64, i64, u8, String, i64)>> {
        let mut sql = String::from(
            "SELECT id, ring_timestamp, tag, decoded_json, captured_unix FROM events \
             WHERE serial = ?1 AND decoded_json IS NOT NULL",
        );
        if let Some(tags) = tags {
            if tags.is_empty() {
                return Ok(Vec::new());
            }
            let list: Vec<String> = tags.iter().map(|t| t.to_string()).collect();
            sql.push_str(&format!(" AND tag IN ({})", list.join(",")));
        }
        if let Some(after) = captured_after {
            sql.push_str(&format!(" AND captured_unix > {after}"));
        }
        sql.push_str(" ORDER BY captured_unix, id");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![serial], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)? as u8,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Decoded events for `serial` whose ring timestamp is in `from_ds..to_ds`
    /// (deciseconds, end not included), as `(id, ring_timestamp, tag,
    /// decoded_json, captured_unix)` in ring-time order, optionally restricted to
    /// `tags`.
    ///
    /// The ring clock restarts after a factory reset, so one range can match
    /// events from more than one clock epoch. Use `captured_unix` to tell them
    /// apart.
    #[allow(clippy::type_complexity)]
    pub fn decoded_events_in_ring_range(
        &self,
        serial: &str,
        tags: Option<&[u8]>,
        from_ds: i64,
        to_ds: i64,
    ) -> Result<Vec<(i64, i64, u8, String, i64)>> {
        let mut sql = String::from(
            "SELECT id, ring_timestamp, tag, decoded_json, captured_unix FROM events \
             WHERE serial = ?1 AND ring_timestamp >= ?2 AND ring_timestamp < ?3 \
             AND decoded_json IS NOT NULL",
        );
        if let Some(tags) = tags {
            if tags.is_empty() {
                return Ok(Vec::new());
            }
            let list: Vec<String> = tags.iter().map(|t| t.to_string()).collect();
            sql.push_str(&format!(" AND tag IN ({})", list.join(",")));
        }
        sql.push_str(" ORDER BY ring_timestamp, id");
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt
            .query_map(params![serial, from_ds, to_ds], |r| {
                Ok((
                    r.get::<_, i64>(0)?,
                    r.get::<_, i64>(1)?,
                    r.get::<_, i64>(2)? as u8,
                    r.get::<_, String>(3)?,
                    r.get::<_, i64>(4)?,
                ))
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Capture time of the newest stored event for `serial`, if any.
    pub fn newest_captured_unix(&self, serial: &str) -> Result<Option<i64>> {
        let v: Option<i64> = self.conn.query_row(
            "SELECT MAX(captured_unix) FROM events WHERE serial = ?1",
            params![serial],
            |r| r.get(0),
        )?;
        Ok(v)
    }

    /// Distinct device serials that have stored events.
    pub fn device_serials(&self) -> Result<Vec<String>> {
        let mut stmt = self
            .conn
            .prepare("SELECT DISTINCT serial FROM events ORDER BY serial")?;
        let rows = stmt
            .query_map([], |r| r.get(0))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Count stored events grouped by event name (descending).
    pub fn event_counts(&self, serial: &str) -> Result<Vec<(String, i64)>> {
        let mut stmt = self.conn.prepare(
            "SELECT name, COUNT(*) FROM events WHERE serial = ?1 GROUP BY name ORDER BY 2 DESC",
        )?;
        let rows = stmt
            .query_map(params![serial], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_event() -> RingEvent {
        RingEvent {
            tag: 0x43,
            name: "debug_event",
            timestamp: 42,
            body: vec![1, 2, 3],
            decoded: None,
        }
    }

    #[test]
    fn full_database_keeps_the_previous_checkpoint() {
        let store = Store::open_in_memory().unwrap();
        store.set_cursor("S1", 7).unwrap();
        let pages: u32 = store
            .conn
            .query_row("PRAGMA page_count", [], |r| r.get(0))
            .unwrap();
        store
            .conn
            .execute_batch(&format!("PRAGMA max_page_count={pages};"))
            .unwrap();
        let mut event = sample_event();
        event.body = vec![42; 1024 * 1024];
        let error = store.commit_batch("S1", &[event], 43).unwrap_err();
        assert!(matches!(
            error,
            crate::error::Error::Sqlite { code: 13, .. }
        ));
        assert_eq!(store.cursor("S1").unwrap(), 7);
        assert!(store.event_counts("S1").unwrap().is_empty());
    }

    #[test]
    fn failed_cursor_commit_rolls_back_entire_batch() {
        let store = Store::open_in_memory().unwrap();
        store.set_cursor("S1", 7).unwrap();
        store.conn.execute_batch("CREATE TRIGGER fail_cursor BEFORE UPDATE ON sync_state BEGIN SELECT RAISE(ABORT, 'injected cursor failure'); END;").unwrap();
        let error = store.commit_batch("S1", &[sample_event()], 43).unwrap_err();
        assert!(matches!(
            error,
            crate::error::Error::Sqlite { code: 19, .. }
        ));
        assert_eq!(store.cursor("S1").unwrap(), 7);
        assert!(store.event_counts("S1").unwrap().is_empty());
        store
            .conn
            .execute_batch("DROP TRIGGER fail_cursor;")
            .unwrap();
        assert_eq!(store.commit_batch("S1", &[sample_event()], 43).unwrap(), 1);
        assert_eq!(store.commit_batch("S1", &[sample_event()], 43).unwrap(), 0);
        assert_eq!(store.cursor("S1").unwrap(), 43);
    }

    #[test]
    fn failed_insert_rolls_back_earlier_rows_and_cursor() {
        let store = Store::open_in_memory().unwrap();
        store.conn.execute_batch("CREATE TRIGGER fail_row BEFORE INSERT ON events WHEN NEW.ring_timestamp=99 BEGIN SELECT RAISE(ABORT, 'injected insert failure'); END;").unwrap();
        let mut bad = sample_event();
        bad.timestamp = 99;
        assert!(store
            .commit_batch("S1", &[sample_event(), bad], 100)
            .is_err());
        assert!(store.event_counts("S1").unwrap().is_empty());
        assert_eq!(store.cursor("S1").unwrap(), 0);
    }

    #[test]
    fn read_only_open_does_not_initialize_schema_or_create_file() {
        let path = std::env::temp_dir().join(format!("oura-missing-{}.db", std::process::id()));
        let _ = std::fs::remove_file(&path);
        assert!(Store::open_read_only(&path).is_err());
        assert!(!path.exists());
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("CREATE TABLE sentinel(value);").unwrap();
        }
        let reader = Store::open_read_only(&path).unwrap();
        assert!(reader.event_counts("S1").is_err());
        assert_eq!(reader.integrity_check().unwrap(), "ok");
        drop(reader);
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn open_enables_wal_on_writable_file() {
        let dir = std::env::temp_dir().join(format!("oura-store-wal-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("wal.db");
        let _ = std::fs::remove_file(&path);
        let store = Store::open(&path).unwrap();
        let mode: String = store
            .conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(mode, "wal");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn reader_survives_open_writer_transaction() {
        let dir = std::env::temp_dir().join(format!("oura-store-rw-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("shared.db");
        let _ = std::fs::remove_file(&path);
        let writer = Store::open(&path).unwrap();
        writer.insert_event("S1", &sample_event()).unwrap();
        // Hold an uncommitted write open — under WAL a reader still gets a
        // consistent snapshot instead of SQLITE_BUSY / a partial read.
        writer.conn.execute_batch("BEGIN IMMEDIATE;").unwrap();
        writer
            .conn
            .execute(
                "INSERT INTO readings (serial, kind, value, unit, captured_unix)
                 VALUES ('S1', 'battery_percent', 50.0, '%', 0)",
                [],
            )
            .unwrap();
        let reader = Store::open_read_only(&path).unwrap();
        let counts = reader.event_counts("S1").unwrap();
        assert_eq!(counts, vec![("debug_event".to_string(), 1)]);
        writer.conn.execute_batch("COMMIT;").unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn events_dedup_and_cursor_roundtrip() {
        let store = Store::open_in_memory().unwrap();
        let ev = RingEvent {
            tag: 0x43,
            name: "debug_event",
            timestamp: 42,
            body: vec![1, 2, 3],
            decoded: None,
        };
        assert!(store.insert_event("S1", &ev).unwrap());
        assert!(!store.insert_event("S1", &ev).unwrap()); // duplicate ignored

        store.set_cursor("S1", 1234).unwrap();
        assert_eq!(store.cursor("S1").unwrap(), 1234);

        let counts = store.event_counts("S1").unwrap();
        assert_eq!(counts, vec![("debug_event".to_string(), 1)]);
    }

    #[test]
    fn fresh_store_is_at_current_schema_version() {
        let store = Store::open_in_memory().unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        let idx: i64 = store
            .conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='index' AND name='idx_events_serial_captured'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(idx, 1);
    }

    #[test]
    fn version_2_file_gets_the_daily_table() {
        let dir = std::env::temp_dir().join(format!("oura-store-v2-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("v2.db");
        let _ = std::fs::remove_file(&path);
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(SCHEMA).unwrap();
            conn.execute_batch("PRAGMA user_version = 2;").unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), 3);
        store.upsert_daily("S1", &[("2026-09-27", "hrv_ms", 41.0)]).unwrap();
        assert_eq!(store.daily_range("S1", "2026-09-27", "2026-09-27").unwrap().len(), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn daily_metrics_upsert_and_read_by_day_range() {
        let store = Store::open_in_memory().unwrap();
        let written = store
            .upsert_daily(
                "S1",
                &[
                    ("2026-09-26", "hrv_ms", 40.0),
                    ("2026-09-26", "rhr", 52.0),
                    ("2026-09-27", "hrv_ms", 44.0),
                    ("2026-09-27", "bad", f64::NAN),
                ],
            )
            .unwrap();
        assert_eq!(written, 3);
        store.upsert_daily("S1", &[("2026-09-26", "hrv_ms", 42.0)]).unwrap();
        store.upsert_daily("S2", &[("2026-09-26", "hrv_ms", 99.0)]).unwrap();
        let rows = store.daily_range("S1", "2026-09-26", "2026-09-27").unwrap();
        assert_eq!(
            rows,
            [
                ("2026-09-26".to_string(), "hrv_ms".to_string(), 42.0),
                ("2026-09-26".to_string(), "rhr".to_string(), 52.0),
                ("2026-09-27".to_string(), "hrv_ms".to_string(), 44.0),
            ]
        );
        assert_eq!(store.daily_range("S1", "2026-09-27", "2026-09-30").unwrap().len(), 1);
        assert_eq!(store.clear_daily("S1").unwrap(), 3);
        assert_eq!(store.daily_range("S2", "2026-09-01", "2026-09-30").unwrap().len(), 1);
    }

    #[test]
    fn ring_range_and_readings_queries() {
        let store = Store::open_in_memory().unwrap();
        let mk = |tag: u8, ts: u32| RingEvent {
            tag,
            name: oura_protocol::events::event_name(tag),
            timestamp: ts,
            body: vec![tag, ts as u8],
            decoded: Some(serde_json::json!({"ts": ts})),
        };
        for (tag, ts) in [(0x5d, 100), (0x47, 150), (0x5d, 200), (0x5d, 300)] {
            store.insert_event("S1", &mk(tag, ts)).unwrap();
        }
        let hits = store
            .decoded_events_in_ring_range("S1", Some(&[0x5d]), 100, 300)
            .unwrap();
        assert_eq!(hits.iter().map(|h| h.1).collect::<Vec<_>>(), [100, 200]);
        assert_eq!(
            store.decoded_events_in_ring_range("S1", None, 0, 1_000).unwrap().len(),
            4
        );
        let battery = Battery { percent: 61, charging_progress: 12, charging_recommended: 0 };
        store.insert_battery("S1", &battery).unwrap();
        let levels = store.readings("S1", "battery_percent", None).unwrap();
        assert_eq!(levels.len(), 1);
        assert_eq!(levels[0].1, 61.0);
        let progress = store.readings("S1", "battery_charging_progress", None).unwrap();
        assert_eq!(progress[0].1, 12.0);
        assert!(store.readings("S1", "battery_percent", Some(i64::MAX - 1)).unwrap().is_empty());
    }

    #[test]
    fn legacy_file_migrates_in_place_and_newer_file_is_refused() {
        let dir = std::env::temp_dir().join(format!("oura-store-mig-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("legacy.db");
        let _ = std::fs::remove_file(&path);
        {
            // A pre-versioning file: tables only, user_version 0.
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch(SCHEMA).unwrap();
        }
        let store = Store::open(&path).unwrap();
        assert_eq!(store.schema_version().unwrap(), SCHEMA_VERSION);
        drop(store);
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("PRAGMA user_version = 99;").unwrap();
        }
        let err = match Store::open(&path) {
            Err(e) => e,
            Ok(_) => panic!("a newer schema must be refused"),
        };
        assert!(matches!(err, Error::SchemaTooNew { found: 99, .. }), "{err}");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn event_times_and_filters_follow_capture_order() {
        let store = Store::open_in_memory().unwrap();
        let mk = |tag: u8, ts: u32, body: u8| RingEvent {
            tag,
            name: oura_protocol::events::event_name(tag),
            timestamp: ts,
            body: vec![body],
            decoded: Some(serde_json::json!({"n": body})),
        };
        store.insert_event_at("S1", &mk(0x42, 5_000_000, 1), 1_000).unwrap();
        store.insert_event_at("S1", &mk(0x85, 10, 2), 2_000).unwrap();
        store.insert_event_at("S1", &mk(0x42, 20, 3), 3_000).unwrap();
        let times: Vec<(i64, i64)> = store
            .event_times("S1")
            .unwrap()
            .into_iter()
            .map(|(ds, cap, _)| (ds, cap))
            .collect();
        assert_eq!(times, [(5_000_000, 1_000), (10, 2_000), (20, 3_000)]);
        let only_42 = store
            .decoded_events_filtered("S1", Some(&[0x42]), None)
            .unwrap();
        assert_eq!(only_42.len(), 2);
        let more = [(mk(0x42, 30, 4), 4_000), (mk(0x42, 20, 3), 4_000)];
        assert_eq!(store.insert_events_at("S1", &more).unwrap(), 1);
        let recent = store
            .decoded_events_filtered("S1", None, Some(1_500))
            .unwrap();
        assert_eq!(recent.len(), 3);
        assert_eq!(store.newest_captured_unix("S1").unwrap(), Some(4_000));
        assert_eq!(store.newest_captured_unix("nope").unwrap(), None);
        store.set_cursor("S1", 77).unwrap();
        store.reset_cursor("S1").unwrap();
        assert_eq!(store.cursor("S1").unwrap(), 0);
    }

    #[test]
    fn decoded_events_preserve_capture_order_across_clock_reset() {
        let store = Store::open_in_memory().unwrap();
        for timestamp in [5_000_000, 10] {
            let event = RingEvent {
                tag: 0x42,
                name: "time_sync",
                timestamp,
                body: vec![0, 0, 0, 0],
                decoded: Some(serde_json::json!({"unix_time": 1_700_000_000})),
            };
            assert!(store.insert_event("S1", &event).unwrap());
        }
        let timestamps: Vec<i64> = store
            .decoded_events()
            .unwrap()
            .into_iter()
            .map(|row| row.0)
            .collect();
        assert_eq!(timestamps, [5_000_000, 10]);
    }
}
