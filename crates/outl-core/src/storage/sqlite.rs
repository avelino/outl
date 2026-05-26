//! SQLite-backed implementation of the [`Storage`] trait.
//!
//! This is the **only** module in `outl-core` allowed to depend on
//! `rusqlite`. Schema definitions, WAL configuration, and bincode
//! serialization for `Op` payloads live here.
//!
//! See `docs/storage.md` §SQLite for the schema rationale.

use crate::hlc::Hlc;
use crate::id::{ActorId, NodeId};
use crate::op::{LogOp, Op};
use crate::storage::{Snapshot, Storage, StorageError};
use parking_lot::Mutex;
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;
use std::path::Path;

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS ops (
    ts_physical INTEGER NOT NULL,
    ts_logical  INTEGER NOT NULL,
    ts_actor    BLOB    NOT NULL,
    actor       BLOB    NOT NULL,
    node_id     BLOB    NOT NULL,
    op_kind     TEXT    NOT NULL,
    op_data     BLOB    NOT NULL,
    PRIMARY KEY (ts_physical, ts_logical, ts_actor)
);

CREATE INDEX IF NOT EXISTS ops_node     ON ops (node_id);
CREATE INDEX IF NOT EXISTS ops_actor    ON ops (actor);
CREATE INDEX IF NOT EXISTS ops_ts_phys  ON ops (ts_physical, ts_logical, ts_actor);

CREATE TABLE IF NOT EXISTS snapshots (
    snapshot_id INTEGER PRIMARY KEY AUTOINCREMENT,
    state_blob  BLOB    NOT NULL,
    created_at  INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS meta (
    key   TEXT PRIMARY KEY,
    value BLOB NOT NULL
);
"#;

/// Convert any rusqlite error into our domain error type.
fn rusqlite_err(e: rusqlite::Error) -> StorageError {
    StorageError::Backend(e.to_string())
}

/// SQLite-backed storage.
///
/// The connection is held behind a `Mutex` so all access is serialized;
/// SQLite's own mutex would suffice but parking_lot is faster and gives us
/// `&self` mutation through interior mutability, which is what the
/// `Storage` trait expects from concurrent readers.
pub struct SqliteStorage {
    conn: Mutex<Connection>,
}

impl SqliteStorage {
    /// Open or create a SQLite database at the given path.
    ///
    /// Enables WAL mode (concurrent readers) and applies the schema.
    pub fn open(path: &Path) -> Result<Self, StorageError> {
        let conn = Connection::open(path).map_err(rusqlite_err)?;
        Self::init(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Open an in-memory SQLite database. Useful for tests and ephemeral
    /// workspaces.
    pub fn open_in_memory() -> Result<Self, StorageError> {
        let conn = Connection::open_in_memory().map_err(rusqlite_err)?;
        Self::init(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Run SQLite `PRAGMA integrity_check` and return the result
    /// string. `"ok"` means the database is healthy; any other value
    /// is a corruption report.
    ///
    /// Exposed at the type level (not on the `Storage` trait) because
    /// it's SQLite-specific. The CLI's `outl doctor` calls it; other
    /// callers shouldn't need to.
    pub fn integrity_check(&self) -> Result<String, StorageError> {
        let conn = self.conn.lock();
        conn.query_row("PRAGMA integrity_check", [], |row| row.get::<_, String>(0))
            .map_err(rusqlite_err)
    }

    fn init(conn: &Connection) -> Result<(), StorageError> {
        // WAL: concurrent readers while a writer is appending an op.
        // For in-memory databases this is a no-op silently.
        let _ = conn.pragma_update(None, "journal_mode", "WAL");
        conn.pragma_update(None, "synchronous", "NORMAL")
            .map_err(rusqlite_err)?;
        conn.pragma_update(None, "foreign_keys", "ON")
            .map_err(rusqlite_err)?;
        conn.execute_batch(SCHEMA).map_err(rusqlite_err)?;
        Ok(())
    }
}

fn serialize_op(op: &Op) -> Result<Vec<u8>, StorageError> {
    // `legacy()` keeps wire-compat with bincode 1.x (fixed-width ints,
    // little-endian, no limit) so existing op logs on disk stay readable
    // after the 1.x → 2.x bump.
    bincode::serde::encode_to_vec(op, bincode::config::legacy())
        .map_err(|e| StorageError::Serialize(e.to_string()))
}

fn deserialize_op(bytes: &[u8]) -> Result<Op, StorageError> {
    let (op, _) = bincode::serde::decode_from_slice(bytes, bincode::config::legacy())
        .map_err(|e| StorageError::Serialize(e.to_string()))?;
    Ok(op)
}

fn op_kind(op: &Op) -> &'static str {
    match op {
        Op::Move { .. } => "Move",
        Op::Edit { .. } => "Edit",
        Op::SetProp { .. } => "SetProp",
        Op::Create { .. } => "Create",
    }
}

fn op_node_id(op: &Op) -> NodeId {
    match op {
        Op::Move { node, .. }
        | Op::Edit { node, .. }
        | Op::SetProp { node, .. }
        | Op::Create { node, .. } => *node,
    }
}

fn ulid_to_bytes(u: ulid::Ulid) -> [u8; 16] {
    u.0.to_be_bytes()
}

fn bytes_to_ulid(b: &[u8]) -> Result<ulid::Ulid, StorageError> {
    let arr: [u8; 16] = b.try_into().map_err(|_| {
        StorageError::Backend(format!(
            "invalid ULID byte length: expected 16, got {}",
            b.len()
        ))
    })?;
    Ok(ulid::Ulid(u128::from_be_bytes(arr)))
}

fn row_to_log_op(row: &rusqlite::Row<'_>) -> rusqlite::Result<LogOp> {
    let physical: i64 = row.get(0)?;
    let logical: i64 = row.get(1)?;
    let ts_actor_bytes: Vec<u8> = row.get(2)?;
    let actor_bytes: Vec<u8> = row.get(3)?;
    let op_blob: Vec<u8> = row.get(6)?;

    let ts_actor = bytes_to_ulid(&ts_actor_bytes).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(2, rusqlite::types::Type::Blob, Box::new(e))
    })?;
    let actor_ulid = bytes_to_ulid(&actor_bytes).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(3, rusqlite::types::Type::Blob, Box::new(e))
    })?;
    let op = deserialize_op(&op_blob).map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(6, rusqlite::types::Type::Blob, Box::new(e))
    })?;

    Ok(LogOp {
        ts: Hlc::new(physical as u64, logical as u32, ActorId(ts_actor)),
        actor: ActorId(actor_ulid),
        op,
    })
}

impl Storage for SqliteStorage {
    fn append_op(&mut self, op: &LogOp) -> Result<(), StorageError> {
        let conn = self.conn.lock();
        let payload = serialize_op(&op.op)?;
        let kind = op_kind(&op.op);
        let node = op_node_id(&op.op);

        conn.execute(
            "INSERT OR IGNORE INTO ops \
              (ts_physical, ts_logical, ts_actor, actor, node_id, op_kind, op_data) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                op.ts.physical_ms as i64,
                op.ts.logical as i64,
                ulid_to_bytes(op.ts.actor.0),
                ulid_to_bytes(op.actor.0),
                ulid_to_bytes(node.0),
                kind,
                payload,
            ],
        )
        .map_err(rusqlite_err)?;
        Ok(())
    }

    fn ops_since(&self, ts: Hlc) -> Result<Vec<LogOp>, StorageError> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT ts_physical, ts_logical, ts_actor, actor, node_id, op_kind, op_data \
                 FROM ops \
                 WHERE (ts_physical, ts_logical, ts_actor) > (?1, ?2, ?3) \
                 ORDER BY ts_physical, ts_logical, ts_actor",
            )
            .map_err(rusqlite_err)?;
        let iter = stmt
            .query_map(
                params![
                    ts.physical_ms as i64,
                    ts.logical as i64,
                    ulid_to_bytes(ts.actor.0),
                ],
                row_to_log_op,
            )
            .map_err(rusqlite_err)?;
        let mut out = Vec::new();
        for row in iter {
            out.push(row.map_err(rusqlite_err)?);
        }
        Ok(out)
    }

    fn ops_for_node(&self, id: NodeId) -> Result<Vec<LogOp>, StorageError> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT ts_physical, ts_logical, ts_actor, actor, node_id, op_kind, op_data \
                 FROM ops WHERE node_id = ?1 \
                 ORDER BY ts_physical, ts_logical, ts_actor",
            )
            .map_err(rusqlite_err)?;
        let iter = stmt
            .query_map(params![ulid_to_bytes(id.0)], row_to_log_op)
            .map_err(rusqlite_err)?;
        let mut out = Vec::new();
        for row in iter {
            out.push(row.map_err(rusqlite_err)?);
        }
        Ok(out)
    }

    fn ops_for_actor(&self, id: ActorId) -> Result<Vec<LogOp>, StorageError> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT ts_physical, ts_logical, ts_actor, actor, node_id, op_kind, op_data \
                 FROM ops WHERE actor = ?1 \
                 ORDER BY ts_physical, ts_logical, ts_actor",
            )
            .map_err(rusqlite_err)?;
        let iter = stmt
            .query_map(params![ulid_to_bytes(id.0)], row_to_log_op)
            .map_err(rusqlite_err)?;
        let mut out = Vec::new();
        for row in iter {
            out.push(row.map_err(rusqlite_err)?);
        }
        Ok(out)
    }

    fn last_ts_per_actor(&self) -> Result<HashMap<ActorId, Hlc>, StorageError> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT actor, MAX(ts_physical), ts_logical, ts_actor \
                 FROM ops GROUP BY actor",
            )
            .map_err(rusqlite_err)?;
        let iter = stmt
            .query_map([], |row| {
                let actor_bytes: Vec<u8> = row.get(0)?;
                let physical: i64 = row.get(1)?;
                let logical: i64 = row.get(2)?;
                let ts_actor_bytes: Vec<u8> = row.get(3)?;
                let actor = bytes_to_ulid(&actor_bytes).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        0,
                        rusqlite::types::Type::Blob,
                        Box::new(e),
                    )
                })?;
                let ts_actor = bytes_to_ulid(&ts_actor_bytes).map_err(|e| {
                    rusqlite::Error::FromSqlConversionFailure(
                        3,
                        rusqlite::types::Type::Blob,
                        Box::new(e),
                    )
                })?;
                Ok((
                    ActorId(actor),
                    Hlc::new(physical as u64, logical as u32, ActorId(ts_actor)),
                ))
            })
            .map_err(rusqlite_err)?;
        let mut out = HashMap::new();
        for row in iter {
            let (a, h) = row.map_err(rusqlite_err)?;
            out.insert(a, h);
        }
        Ok(out)
    }

    fn all_ops(&self) -> Result<Vec<LogOp>, StorageError> {
        let conn = self.conn.lock();
        let mut stmt = conn
            .prepare(
                "SELECT ts_physical, ts_logical, ts_actor, actor, node_id, op_kind, op_data \
                 FROM ops ORDER BY ts_physical, ts_logical, ts_actor",
            )
            .map_err(rusqlite_err)?;
        let iter = stmt.query_map([], row_to_log_op).map_err(rusqlite_err)?;
        let mut out = Vec::new();
        for row in iter {
            out.push(row.map_err(rusqlite_err)?);
        }
        Ok(out)
    }

    fn save_snapshot(&mut self, snapshot: &Snapshot) -> Result<(), StorageError> {
        let conn = self.conn.lock();
        let created_at = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as i64)
            .unwrap_or(0);
        conn.execute(
            "INSERT INTO snapshots (state_blob, created_at) VALUES (?1, ?2)",
            params![snapshot.bytes, created_at],
        )
        .map_err(rusqlite_err)?;
        Ok(())
    }

    fn load_snapshot(&self) -> Result<Option<Snapshot>, StorageError> {
        let conn = self.conn.lock();
        let row: Option<Vec<u8>> = conn
            .query_row(
                "SELECT state_blob FROM snapshots ORDER BY snapshot_id DESC LIMIT 1",
                [],
                |row| row.get(0),
            )
            .optional()
            .map_err(rusqlite_err)?;
        Ok(row.map(|bytes| Snapshot { bytes }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fractional::Fractional;
    use crate::op::Op;

    fn make_create(actor: ActorId, physical: u64, node: NodeId) -> LogOp {
        LogOp {
            ts: Hlc::new(physical, 0, actor),
            actor,
            op: Op::Create {
                node,
                parent: NodeId::root(),
                position: Fractional::first(),
            },
        }
    }

    #[test]
    fn append_and_all_ops_roundtrip() {
        let mut s = SqliteStorage::open_in_memory().unwrap();
        let actor = ActorId::new();
        let n1 = NodeId::new();
        let n2 = NodeId::new();
        s.append_op(&make_create(actor, 1, n1)).unwrap();
        s.append_op(&make_create(actor, 2, n2)).unwrap();
        let all = s.all_ops().unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].ts.physical_ms, 1);
        assert_eq!(all[1].ts.physical_ms, 2);
    }

    #[test]
    fn ops_since_filters_strictly() {
        let mut s = SqliteStorage::open_in_memory().unwrap();
        let actor = ActorId::new();
        let n1 = NodeId::new();
        let n2 = NodeId::new();
        s.append_op(&make_create(actor, 1, n1)).unwrap();
        s.append_op(&make_create(actor, 5, n2)).unwrap();
        let cutoff = Hlc::new(1, 0, actor);
        let after = s.ops_since(cutoff).unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].ts.physical_ms, 5);
    }

    #[test]
    fn idempotent_append_via_pk_conflict() {
        let mut s = SqliteStorage::open_in_memory().unwrap();
        let actor = ActorId::new();
        let n = NodeId::new();
        let op = make_create(actor, 1, n);
        s.append_op(&op).unwrap();
        s.append_op(&op).unwrap(); // duplicate by PK; INSERT OR IGNORE
        let all = s.all_ops().unwrap();
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn ops_for_node_and_actor() {
        let mut s = SqliteStorage::open_in_memory().unwrap();
        let actor1 = ActorId::new();
        let actor2 = ActorId::new();
        let n1 = NodeId::new();
        let n2 = NodeId::new();
        s.append_op(&make_create(actor1, 1, n1)).unwrap();
        s.append_op(&make_create(actor1, 2, n2)).unwrap();
        s.append_op(&make_create(actor2, 3, n1)).unwrap();

        let by_node = s.ops_for_node(n1).unwrap();
        assert_eq!(by_node.len(), 2);

        let by_actor = s.ops_for_actor(actor1).unwrap();
        assert_eq!(by_actor.len(), 2);

        let last = s.last_ts_per_actor().unwrap();
        assert_eq!(last[&actor1].physical_ms, 2);
        assert_eq!(last[&actor2].physical_ms, 3);
    }

    #[test]
    fn snapshot_roundtrip() {
        let mut s = SqliteStorage::open_in_memory().unwrap();
        assert!(s.load_snapshot().unwrap().is_none());
        s.save_snapshot(&Snapshot {
            bytes: vec![1, 2, 3, 4],
        })
        .unwrap();
        let loaded = s.load_snapshot().unwrap().unwrap();
        assert_eq!(loaded.bytes, vec![1, 2, 3, 4]);
    }
}
