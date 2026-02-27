use std::fs::{OpenOptions, File};
use std::io::{Write, BufReader, BufRead};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use anyhow::{Result, Context};
use serde::{Deserialize, Serialize};
use crate::models::TraceEvent;

#[derive(Serialize, Deserialize, Debug)]
pub struct WalEntry {
    pub event: TraceEvent,
    pub tenant_id: Option<String>,
}

pub struct WalManager {
    file: Arc<Mutex<File>>,
    path: PathBuf,
}

impl WalManager {
    pub fn new(data_dir: &str) -> Result<Self> {
        let path = Path::new(data_dir).join("ingestion.wal");
        std::fs::create_dir_all(data_dir).context("Failed to create WAL directory")?;
        
        let file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .context("Failed to open WAL file")?;
            
        Ok(Self {
            file: Arc::new(Mutex::new(file)),
            path,
        })
    }

    /// Append an event to the WAL and ensure it's flushed to disk
    pub fn append(&self, event: TraceEvent, tenant_id: Option<String>) -> Result<()> {
        let entry = WalEntry { event, tenant_id };
        let mut json = serde_json::to_vec(&entry)?;
        json.push(b'\n');
        
        crate::metrics::WAL_ENTRIES.inc();
        
        let mut file = self.file.lock().map_err(|_| anyhow::anyhow!("WAL lock poisoned"))?;
        file.write_all(&json)?;
        file.sync_all()?; // Core requirement: durable fsync
        
        Ok(())
    }

    /// Replay all events from the WAL. 
    /// Note: This is usually done on startup.
    pub fn replay(&self) -> Result<Vec<WalEntry>> {
        let file = File::open(&self.path);
        if file.is_err() {
            return Ok(Vec::new()); // No WAL to replay
        }
        
        let reader = BufReader::new(file.unwrap());
        let mut entries = Vec::new();
        
        for line in reader.lines() {
            let line = line?;
            if line.trim().is_empty() { continue; }
            if let Ok(entry) = serde_json::from_str::<WalEntry>(&line) {
                entries.push(entry);
            }
        }
        
        Ok(entries)
    }

    /// Truncate/Clear the WAL once data is safely persisted in the primary DB
    pub fn checkpoint(&self) -> Result<()> {
        let mut file = self.file.lock().map_err(|_| anyhow::anyhow!("WAL lock poisoned"))?;
        file.set_len(0)?;
        file.sync_all()?;
        Ok(())
    }
}
