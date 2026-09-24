//! Replication: copy the raw rows of one store into another, in order, in batches.
//!
//! The phone (or the desktop) exports rows after the ids it last sent. The hub
//! imports them into its own store with the same schema. Events are the lossless
//! record, so a replica can run every report the source can. The sync cursor is
//! not copied: it belongs to the device that talks to the ring.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::storage::{Store, SCHEMA_VERSION};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DeviceRow {
    pub serial: String,
    pub hardware_id: Option<String>,
    pub firmware: Option<String>,
    pub api_version: Option<String>,
    pub mac: Option<String>,
    pub updated_unix: i64,
}

/// One raw event. `body_hex` is the event body after the 4-byte timestamp; the
/// decoded JSON is recomputed on import, so the wire format stays small.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EventRow {
    pub id: i64,
    pub serial: String,
    pub tag: u8,
    pub ring_timestamp: i64,
    pub body_hex: String,
    pub captured_unix: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReadingRow {
    pub id: i64,
    pub serial: String,
    pub kind: String,
    pub value: f64,
    pub unit: Option<String>,
    pub captured_unix: i64,
}

/// One page of rows. `next_event_id` / `next_reading_id` are the ids to pass as
/// `after_*` for the next page; `more` says whether a next page exists.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ExportBatch {
    pub schema_version: i64,
    pub devices: Vec<DeviceRow>,
    pub events: Vec<EventRow>,
    pub readings: Vec<ReadingRow>,
    pub next_event_id: i64,
    pub next_reading_id: i64,
    pub more: bool,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct ImportOutcome {
    pub devices: usize,
    pub events_seen: usize,
    pub events_inserted: usize,
    /// Rows with a body that is not valid hex. They are skipped, never stored.
    pub events_rejected: usize,
    pub readings_seen: usize,
    pub readings_inserted: usize,
    /// The largest event id now in this store.
    pub max_event_id: i64,
}

impl Store {
    /// The largest `(event id, reading id)` in this store; 0 when a table is empty.
    pub fn max_ids(&self) -> Result<(i64, i64)> {
        let e: i64 = self
            .conn
            .query_row("SELECT COALESCE(MAX(id), 0) FROM events", [], |r| r.get(0))?;
        let r: i64 = self
            .conn
            .query_row("SELECT COALESCE(MAX(id), 0) FROM readings", [], |r| r.get(0))?;
        Ok((e, r))
    }

    /// Rows after the given ids, oldest first, at most `limit` events and `limit`
    /// readings. Every device row is always included: they are few and idempotent.
    pub fn export_after(&self, after_event_id: i64, after_reading_id: i64, limit: usize) -> Result<ExportBatch> {
        let limit = limit.max(1) as i64;
        let devices = {
            let mut stmt = self.conn.prepare(
                "SELECT serial, hardware_id, firmware, api_version, mac, updated_unix FROM device ORDER BY serial",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok(DeviceRow {
                    serial: r.get(0)?,
                    hardware_id: r.get(1)?,
                    firmware: r.get(2)?,
                    api_version: r.get(3)?,
                    mac: r.get(4)?,
                    updated_unix: r.get(5)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
            rows
        };
        let mut events = {
            let mut stmt = self.conn.prepare(
                "SELECT id, serial, tag, ring_timestamp, body, captured_unix FROM events
                 WHERE id > ?1 ORDER BY id LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![after_event_id, limit + 1], |r| {
                Ok(EventRow {
                    id: r.get(0)?,
                    serial: r.get(1)?,
                    tag: r.get::<_, i64>(2)? as u8,
                    ring_timestamp: r.get(3)?,
                    body_hex: hex::encode(r.get::<_, Vec<u8>>(4)?),
                    captured_unix: r.get(5)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
            rows
        };
        let mut readings = {
            let mut stmt = self.conn.prepare(
                "SELECT id, serial, kind, value, unit, captured_unix FROM readings
                 WHERE id > ?1 ORDER BY id LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![after_reading_id, limit + 1], |r| {
                Ok(ReadingRow {
                    id: r.get(0)?,
                    serial: r.get(1)?,
                    kind: r.get(2)?,
                    value: r.get(3)?,
                    unit: r.get(4)?,
                    captured_unix: r.get(5)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
            rows
        };
        let more_events = events.len() as i64 > limit;
        let more_readings = readings.len() as i64 > limit;
        events.truncate(limit as usize);
        readings.truncate(limit as usize);
        Ok(ExportBatch {
            schema_version: SCHEMA_VERSION,
            next_event_id: events.last().map(|e| e.id).unwrap_or(after_event_id),
            next_reading_id: readings.last().map(|r| r.id).unwrap_or(after_reading_id),
            more: more_events || more_readings,
            devices,
            events,
            readings,
        })
    }

    /// Insert a batch. Duplicates are ignored: events by their natural key
    /// `(serial, tag, ring_timestamp, body)`, readings by `(serial, kind, value,
    /// captured_unix)`. Ids are not copied; this store assigns its own.
    pub fn import_batch(&self, batch: &ExportBatch) -> Result<ImportOutcome> {
        let mut out = ImportOutcome::default();
        let tx = self.conn.unchecked_transaction()?;
        for d in &batch.devices {
            tx.execute(
                "INSERT INTO device (serial, hardware_id, firmware, api_version, mac, updated_unix)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)
                 ON CONFLICT(serial) DO UPDATE SET
                   hardware_id=COALESCE(excluded.hardware_id, device.hardware_id),
                   firmware=COALESCE(excluded.firmware, device.firmware),
                   api_version=COALESCE(excluded.api_version, device.api_version),
                   mac=COALESCE(excluded.mac, device.mac),
                   updated_unix=MAX(excluded.updated_unix, device.updated_unix)",
                params![d.serial, d.hardware_id, d.firmware, d.api_version, d.mac, d.updated_unix],
            )?;
            out.devices += 1;
        }
        for e in &batch.events {
            out.events_seen += 1;
            let Ok(body) = hex::decode(&e.body_hex) else {
                out.events_rejected += 1;
                continue;
            };
            let decoded = oura_protocol::events::decode_event_body(e.tag, &body)
                .map(|v| serde_json::to_string(&v).unwrap_or_default());
            let name = oura_protocol::events::event_name(e.tag);
            let changed = tx.execute(
                "INSERT OR IGNORE INTO events
                   (serial, tag, name, ring_timestamp, body, decoded_json, captured_unix)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![e.serial, e.tag as i64, name, e.ring_timestamp, body, decoded, e.captured_unix],
            )?;
            out.events_inserted += changed;
        }
        for r in &batch.readings {
            out.readings_seen += 1;
            let changed = tx.execute(
                "INSERT INTO readings (serial, kind, value, unit, captured_unix)
                 SELECT ?1, ?2, ?3, ?4, ?5
                 WHERE NOT EXISTS (SELECT 1 FROM readings
                                   WHERE serial = ?1 AND kind = ?2 AND value = ?3 AND captured_unix = ?5)",
                params![r.serial, r.kind, r.value, r.unit, r.captured_unix],
            )?;
            out.readings_inserted += changed;
        }
        tx.commit()?;
        out.max_event_id = self
            .conn
            .query_row("SELECT COALESCE(MAX(id), 0) FROM events", [], |r| r.get(0))
            .optional()?
            .unwrap_or(0);
        Ok(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use oura_protocol::events::RingEvent;

    fn ev(ts: u32, body: Vec<u8>) -> RingEvent {
        RingEvent { tag: 0x41, name: "debug_event", timestamp: ts, body, decoded: None }
    }

    fn seeded() -> Store {
        let s = Store::open_in_memory().unwrap();
        s.upsert_device("S1", Some("HW"), None).unwrap();
        for i in 0..5u32 {
            s.insert_event_at("S1", &ev(i * 10, vec![i as u8, 0xff]), 1_000 + i as i64).unwrap();
        }
        s.insert_reading("S1", "battery", 61.0, "%").unwrap();
        s.insert_reading("S1", "hr", 62.0, "bpm").unwrap();
        s
    }

    #[test]
    fn export_pages_in_id_order() {
        let s = seeded();
        let p1 = s.export_after(0, 0, 2).unwrap();
        assert_eq!(p1.schema_version, SCHEMA_VERSION);
        assert_eq!(p1.devices.len(), 1);
        assert_eq!(p1.events.iter().map(|e| e.id).collect::<Vec<_>>(), [1, 2]);
        assert_eq!(p1.events[1].body_hex, "01ff");
        assert_eq!(p1.readings.len(), 2);
        assert!(p1.more);
        assert_eq!((p1.next_event_id, p1.next_reading_id), (2, 2));
        let p2 = s.export_after(p1.next_event_id, p1.next_reading_id, 2).unwrap();
        assert_eq!(p2.events.iter().map(|e| e.id).collect::<Vec<_>>(), [3, 4]);
        assert!(p2.readings.is_empty());
        assert!(p2.more);
        let p3 = s.export_after(p2.next_event_id, p2.next_reading_id, 2).unwrap();
        assert_eq!(p3.events.iter().map(|e| e.id).collect::<Vec<_>>(), [5]);
        assert!(!p3.more);
        assert_eq!(p3.next_event_id, 5);
        let empty = s.export_after(5, 2, 2).unwrap();
        assert!(empty.events.is_empty() && !empty.more);
        assert_eq!((empty.next_event_id, empty.next_reading_id), (5, 2));
    }

    #[test]
    fn import_is_idempotent_and_redecodes() {
        let src = seeded();
        let dst = Store::open_in_memory().unwrap();
        let batch = src.export_after(0, 0, 100).unwrap();
        let out = dst.import_batch(&batch).unwrap();
        assert_eq!(out.devices, 1);
        assert_eq!((out.events_seen, out.events_inserted), (5, 5));
        assert_eq!((out.readings_seen, out.readings_inserted), (2, 2));
        assert_eq!(out.max_event_id, 5);
        let again = dst.import_batch(&batch).unwrap();
        assert_eq!(again.events_inserted, 0);
        assert_eq!(again.readings_inserted, 0);
        assert_eq!(dst.event_counts("S1").unwrap(), vec![("ring_start".to_string(), 5)]); // the name is recomputed from the tag
        assert_eq!(dst.max_ids().unwrap(), (5, 2));
        let info = dst.device_info().unwrap().unwrap();
        assert_eq!((info.0.as_str(), info.1.as_str()), ("S1", "HW"));
        // the replica exports the same rows back
        let round = dst.export_after(0, 0, 100).unwrap();
        assert_eq!(round.events.iter().map(|e| &e.body_hex).collect::<Vec<_>>(),
                   batch.events.iter().map(|e| &e.body_hex).collect::<Vec<_>>());
    }

    #[test]
    fn import_rejects_bad_hex_and_keeps_the_rest() {
        let dst = Store::open_in_memory().unwrap();
        let mut batch = seeded().export_after(0, 0, 100).unwrap();
        batch.events[2].body_hex = "zz".into();
        let out = dst.import_batch(&batch).unwrap();
        assert_eq!(out.events_rejected, 1);
        assert_eq!(out.events_inserted, 4);
    }

    #[test]
    fn batch_round_trips_through_json() {
        let batch = seeded().export_after(0, 0, 100).unwrap();
        let text = serde_json::to_string(&batch).unwrap();
        let back: ExportBatch = serde_json::from_str(&text).unwrap();
        assert_eq!(back, batch);
    }
}
