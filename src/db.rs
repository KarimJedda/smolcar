use rusqlite::{Connection, params};
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::{Arc, Mutex};

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct EventFilter {
    pub pallet: String,
    pub method: Option<String>, // None means exclude all events from this pallet
}

#[derive(Clone, Serialize, Deserialize, Debug)]
pub struct StoredBlock {
    pub number: u32,
    pub hash: String,
    pub extrinsics: Vec<serde_json::Value>,
    pub timestamp: i64,
}

pub struct Database {
    conn: Arc<Mutex<Connection>>,
    event_filters: Vec<EventFilter>,
    extrinsic_filters: Vec<String>,
}

impl Database {
    pub fn new<P: AsRef<Path>>(
        path: P,
        event_filters: Vec<EventFilter>,
        extrinsic_filters: Vec<String>,
    ) -> Result<Self, rusqlite::Error> {
        let conn = Connection::open(path)?;

        conn.execute(
            "CREATE TABLE IF NOT EXISTS blocks (
                block_number INTEGER PRIMARY KEY,
                block_hash TEXT NOT NULL,
                block_data TEXT NOT NULL,
                timestamp INTEGER NOT NULL
            )",
            [],
        )?;

        // Create index on timestamp for range queries
        conn.execute(
            "CREATE INDEX IF NOT EXISTS idx_timestamp ON blocks(timestamp)",
            [],
        )?;

        Ok(Database {
            conn: Arc::new(Mutex::new(conn)),
            event_filters,
            extrinsic_filters,
        })
    }

    pub fn should_include_event(&self, pallet: &str, method: &str) -> bool {
        for filter in &self.event_filters {
            if filter.pallet == pallet {
                match &filter.method {
                    None => return false, // Exclude all events from this pallet
                    Some(m) if m == method => return false, // Exclude this specific method
                    _ => {}
                }
            }
        }
        true
    }

    pub fn should_include_extrinsic(&self, action: &str) -> bool {
        !self.extrinsic_filters.contains(&action.to_string())
    }

    pub fn store_block(&self, block: &StoredBlock) -> Result<(), rusqlite::Error> {
        let block_data_json = serde_json::to_string(block)
            .map_err(|e| rusqlite::Error::ToSqlConversionFailure(Box::new(e)))?;

        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO blocks (block_number, block_hash, block_data, timestamp)
             VALUES (?1, ?2, ?3, ?4)",
            params![block.number, block.hash, block_data_json, block.timestamp],
        )?;

        Ok(())
    }

    pub fn get_block(&self, block_number: u32) -> Result<Option<StoredBlock>, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT block_data FROM blocks WHERE block_number = ?1"
        )?;

        let mut rows = stmt.query(params![block_number])?;

        if let Some(row) = rows.next()? {
            let block_data_json: String = row.get(0)?;
            let block: StoredBlock = serde_json::from_str(&block_data_json)
                .map_err(|e| rusqlite::Error::FromSqlConversionFailure(
                    0,
                    rusqlite::types::Type::Text,
                    Box::new(e)
                ))?;
            Ok(Some(block))
        } else {
            Ok(None)
        }
    }

    pub fn get_latest_block_number(&self) -> Result<Option<u32>, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT MAX(block_number) FROM blocks")?;
        let mut rows = stmt.query([])?;

        if let Some(row) = rows.next()? {
            Ok(row.get(0)?)
        } else {
            Ok(None)
        }
    }

    pub fn get_blocks_range(&self, start: u32, end: u32, limit: u32) -> Result<Vec<StoredBlock>, rusqlite::Error> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT block_data FROM blocks
             WHERE block_number >= ?1 AND block_number <= ?2
             ORDER BY block_number DESC
             LIMIT ?3"
        )?;

        let rows = stmt.query_map(params![start, end, limit], |row| {
            let block_data_json: String = row.get(0)?;
            Ok(block_data_json)
        })?;

        let mut blocks = Vec::new();
        for row in rows {
            let block_data_json = row?;
            if let Ok(block) = serde_json::from_str(&block_data_json) {
                blocks.push(block);
            }
        }

        Ok(blocks)
    }
}
