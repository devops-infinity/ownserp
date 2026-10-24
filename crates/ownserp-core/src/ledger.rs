use std::collections::BTreeMap;
use std::fs::{File, OpenOptions};
use std::io::{BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

const MAX_LINE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolTotals {
    pub calls: u64,
    pub credits: u64,
    pub tokens: u64,
    pub cache_hits: u64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Totals {
    pub calls: u64,
    pub credits: u64,
    pub tokens: u64,
    pub tokens_saved: u64,
    pub cache_hits: u64,
    pub per_tool: BTreeMap<String, ToolTotals>,
}

#[derive(Debug, Clone, Default)]
pub struct CallRecord {
    pub tool: String,
    pub endpoint: String,
    pub args_hash: String,
    pub credits_estimated: u32,
    pub credits_actual: Option<u32>,
    pub batch_multiplier: u32,
    pub duration_ms: u64,
    pub status: String,
    pub response_bytes: u64,
    pub tokens: u64,
    pub tokens_saved: u64,
    pub cache_hit: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LedgerRow {
    pub seq: u64,
    pub ts_unix: u64,
    pub tool: String,
    pub endpoint: String,
    pub args_hash: String,
    pub credits_estimated: u32,
    pub credits_actual: Option<u32>,
    pub batch_multiplier: u32,
    pub duration_ms: u64,
    pub status: String,
    pub response_bytes: u64,
    pub tokens: u64,
    pub tokens_saved: u64,
    pub cache_hit: bool,
    pub prev: String,
    pub hash: String,
}

struct LedgerState {
    path: PathBuf,
    file: File,
    seq: u64,
    last_hash: String,
    totals: Totals,
    pending: u64,
}

#[derive(Clone)]
pub struct Ledger {
    state: Arc<Mutex<LedgerState>>,
    healthy: Arc<AtomicBool>,
}

pub struct Reservation {
    ledger: Ledger,
    credits: u32,
}

impl Drop for Reservation {
    fn drop(&mut self) {
        self.ledger.settle(self.credits);
    }
}

impl Ledger {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        let path = path.into();
        if let Some(parent) = path.parent() {
            let existed = parent.exists();
            let mut builder = std::fs::DirBuilder::new();
            builder.recursive(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(parent)?;
            if !existed || parent.starts_with(crate::config::default_data_dir()) {
                lock_down_dir(parent)?;
            }
        }
        let mut seq = 0;
        let mut last_hash = String::from("genesis");
        let mut totals = Totals::default();
        let mut previous = String::from("genesis");
        let valid_len = walk_rows(&path, |row| {
            let expected = chain_hash(&previous, row);
            if row.hash != expected || row.prev != previous {
                return Err(Error::Ledger(format!(
                    "hash chain broken at sequence {} in {}",
                    row.seq,
                    path.display()
                )));
            }
            previous = row.hash.clone();
            seq = row.seq;
            last_hash = row.hash.clone();
            accumulate(&mut totals, row);
            Ok(())
        })?;
        if let Ok(metadata) = std::fs::symlink_metadata(&path) {
            if metadata.file_type().is_symlink() {
                return Err(Error::Ledger(format!(
                    "{} is a symbolic link; point the ledger at a regular file",
                    path.display()
                )));
            }
            if !metadata.is_file() {
                return Err(Error::Ledger(format!(
                    "{} is not a regular file",
                    path.display()
                )));
            }
        }
        let mut options = OpenOptions::new();
        options.create(true).read(true).append(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            options.mode(0o600);
        }
        let file = options.open(&path)?;
        file.try_lock().map_err(|error| {
            Error::Ledger(format!(
                "another ownserp process holds {}: {error}",
                path.display()
            ))
        })?;
        lock_down_file(&path)?;
        sync_parent_dir(&path);
        let stored = file.metadata()?.len();
        if stored > valid_len {
            file.set_len(valid_len)?;
            tracing::warn!(
                path = %path.display(),
                removed = stored.saturating_sub(valid_len),
                "truncated a partial final ledger line before appending"
            );
        }
        Ok(Self {
            state: Arc::new(Mutex::new(LedgerState {
                path,
                file,
                seq,
                last_hash,
                totals,
                pending: 0,
            })),
            healthy: Arc::new(AtomicBool::new(true)),
        })
    }

    pub fn append(&self, record: CallRecord) -> Result<LedgerRow> {
        let outcome = self.append_inner(record);
        if outcome.is_err() {
            self.healthy.store(false, Ordering::Relaxed);
        }
        outcome
    }

    fn append_inner(&self, record: CallRecord) -> Result<LedgerRow> {
        let (written, handle) = {
            let mut state = self.lock()?;
            let row = LedgerRow {
                seq: state.seq.saturating_add(1),
                ts_unix: unix_seconds(),
                tool: record.tool,
                endpoint: record.endpoint,
                args_hash: record.args_hash,
                credits_estimated: record.credits_estimated,
                credits_actual: record.credits_actual,
                batch_multiplier: record.batch_multiplier,
                duration_ms: record.duration_ms,
                status: record.status,
                response_bytes: record.response_bytes,
                tokens: record.tokens,
                tokens_saved: record.tokens_saved,
                cache_hit: record.cache_hit,
                prev: state.last_hash.clone(),
                hash: String::new(),
            };
            let hash = chain_hash(&row.prev, &row);
            let written = LedgerRow { hash, ..row };
            let mut line = serde_json::to_string(&written).map_err(|error| Error::Json {
                context: "encoding a ledger row",
                source: error,
            })?;
            line.push('\n');
            if line.len() > MAX_LINE_BYTES {
                return Err(Error::Ledger(format!(
                    "a ledger row of {} bytes passes the {MAX_LINE_BYTES} byte cap",
                    line.len()
                )));
            }
            state.file.write_all(line.as_bytes())?;
            state.seq = written.seq;
            state.last_hash = written.hash.clone();
            accumulate(&mut state.totals, &written);
            let handle = state.file.try_clone().map_err(|error| Error::Io {
                context: "cloning the ledger handle",
                source: error,
            })?;
            (written, handle)
        };
        handle.sync_data().map_err(|error| Error::Io {
            context: "syncing the ledger",
            source: error,
        })?;
        Ok(written)
    }

    pub fn reserve(&self, credits: u32, budget: u64) -> Result<Reservation> {
        if !self.healthy.load(Ordering::Relaxed) {
            return Err(Error::Ledger(
                "the ledger could not record the last call; restore the ledger directory and restart".into(),
            ));
        }
        let mut state = self.lock()?;
        let committed = state.totals.credits.saturating_add(state.pending);
        if committed.saturating_add(u64::from(credits)) > budget {
            return Err(Error::BudgetRefused(format!(
                "{} credits would pass the session budget of {budget}, {} already committed",
                credits, committed
            )));
        }
        state.pending = state.pending.saturating_add(u64::from(credits));
        Ok(Reservation {
            ledger: self.clone(),
            credits,
        })
    }

    fn settle(&self, credits: u32) {
        if let Ok(mut state) = self.state.lock() {
            state.pending = state.pending.saturating_sub(u64::from(credits));
        }
    }

    pub fn totals(&self) -> Result<Totals> {
        let state = self.lock()?;
        Ok(state.totals.clone())
    }

    pub fn pending(&self) -> Result<u64> {
        let state = self.lock()?;
        Ok(state.pending)
    }

    pub fn path(&self) -> Result<PathBuf> {
        let state = self.lock()?;
        Ok(state.path.clone())
    }

    fn lock(&self) -> Result<std::sync::MutexGuard<'_, LedgerState>> {
        self.state
            .lock()
            .map_err(|_| Error::Ledger("ledger lock poisoned".into()))
    }
}

pub fn verify(path: &Path) -> Result<u64> {
    let mut count = 0u64;
    let mut previous = String::from("genesis");
    let _ = walk_rows(path, |row| {
        if row.prev != previous {
            return Err(Error::Ledger(format!(
                "prev link mismatch at sequence {}",
                row.seq
            )));
        }
        let expected = chain_hash(&previous, row);
        if row.hash != expected {
            return Err(Error::Ledger(format!(
                "hash mismatch at sequence {}",
                row.seq
            )));
        }
        previous = row.hash.clone();
        count = count.saturating_add(1);
        Ok(())
    })?;
    Ok(count)
}

pub fn rotate(path: &Path) -> Result<(u64, PathBuf)> {
    if !path.exists() {
        return Err(Error::Ledger(format!(
            "no ledger at {} to rotate",
            path.display()
        )));
    }
    let rows = verify(path)?;
    let lock_file = OpenOptions::new().append(true).open(path)?;
    lock_file.try_lock().map_err(|error| {
        Error::Ledger(format!(
            "stop the running server before rotating {}: {error}",
            path.display()
        ))
    })?;
    let stamp = unix_seconds();
    let parent = path.parent().unwrap_or_else(|| Path::new("."));
    let stem = path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or("calls");
    let mut archive = parent.join(format!("{stem}-{stamp}.jsonl"));
    let mut counter = 1u32;
    while archive.exists() {
        archive = parent.join(format!("{stem}-{stamp}-{counter}.jsonl"));
        counter = counter.saturating_add(1);
    }
    std::fs::rename(path, &archive)?;
    Ok((rows, archive))
}

fn read_line_capped<R: std::io::BufRead>(reader: &mut R) -> std::io::Result<Option<Vec<u8>>> {
    let mut line = Vec::new();
    loop {
        let (used, found) = {
            let available = reader.fill_buf()?;
            if available.is_empty() {
                return Ok(if line.is_empty() { None } else { Some(line) });
            }
            let found = available.iter().position(|byte| *byte == b'\n');
            let used = found.map_or(available.len(), |index| index.saturating_add(1));
            if line.len().saturating_add(used) > MAX_LINE_BYTES {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("a ledger line passes the {MAX_LINE_BYTES} byte cap"),
                ));
            }
            let chunk = match available.get(..used) {
                Some(chunk) => chunk,
                None => available,
            };
            line.extend_from_slice(chunk);
            (used, found.is_some())
        };
        reader.consume(used);
        if found {
            return Ok(Some(line));
        }
    }
}

fn walk_rows<F>(path: &Path, mut visit: F) -> Result<u64>
where
    F: FnMut(&LedgerRow) -> Result<()>,
{
    if !path.exists() {
        return Ok(0);
    }
    let mut reader = BufReader::new(File::open(path)?);
    let mut buffer = Vec::new();
    let mut consumed = 0u64;
    let mut valid_len = 0u64;
    loop {
        buffer.clear();
        let Some(line) = read_line_capped(&mut reader)? else {
            break;
        };
        let read = line.len();
        buffer.extend_from_slice(&line);
        if read == 0 {
            break;
        }
        consumed = consumed.saturating_add(read as u64);
        let terminated = buffer.last() == Some(&b'\n');
        let body = buffer.strip_suffix(b"\n").unwrap_or(buffer.as_slice());
        let text = String::from_utf8_lossy(body);
        if text.trim().is_empty() {
            if terminated {
                valid_len = consumed;
            }
            continue;
        }
        match serde_json::from_str::<LedgerRow>(&text) {
            Ok(row) if terminated => {
                visit(&row)?;
                valid_len = consumed;
            }
            Ok(_) => {
                tracing::warn!(
                    path = %path.display(),
                    "ignoring an unterminated final ledger line left by an interrupted write"
                );
                break;
            }
            Err(error) => {
                if !terminated {
                    tracing::warn!(
                        path = %path.display(),
                        %error,
                        "ignoring a partial final ledger line left by an interrupted write"
                    );
                    break;
                }
                return Err(Error::Ledger(format!("unreadable ledger line: {error}")));
            }
        }
    }
    Ok(valid_len)
}

fn sync_parent_dir(path: &Path) {
    #[cfg(unix)]
    {
        if let Some(parent) = path.parent()
            && let Ok(handle) = File::open(parent)
        {
            let _ = handle.sync_all();
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

#[cfg(unix)]
fn lock_down_dir(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700)).map_err(|error| {
        Error::Ledger(format!(
            "cannot restrict {} to its owner: {error}",
            path.display()
        ))
    })
}

#[cfg(not(unix))]
fn lock_down_dir(_path: &Path) -> Result<()> {
    Ok(())
}

#[cfg(unix)]
fn lock_down_file(path: &Path) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(|error| {
        Error::Ledger(format!(
            "cannot restrict {} to its owner: {error}",
            path.display()
        ))
    })
}

#[cfg(not(unix))]
fn lock_down_file(_path: &Path) -> Result<()> {
    Ok(())
}

fn accumulate(totals: &mut Totals, row: &LedgerRow) {
    let credits = u64::from(row.credits_actual.unwrap_or(row.credits_estimated));
    totals.calls = totals.calls.saturating_add(1);
    totals.credits = totals.credits.saturating_add(credits);
    totals.tokens = totals.tokens.saturating_add(row.tokens);
    totals.tokens_saved = totals.tokens_saved.saturating_add(row.tokens_saved);
    if row.cache_hit {
        totals.cache_hits = totals.cache_hits.saturating_add(1);
    }
    let entry = totals.per_tool.entry(row.tool.clone()).or_default();
    entry.calls = entry.calls.saturating_add(1);
    entry.credits = entry.credits.saturating_add(credits);
    entry.tokens = entry.tokens.saturating_add(row.tokens);
    if row.cache_hit {
        entry.cache_hits = entry.cache_hits.saturating_add(1);
    }
}

fn chain_hash(prev: &str, row: &LedgerRow) -> String {
    let payload = format!(
        "{prev}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}\u{1f}{}",
        row.seq,
        row.ts_unix,
        row.tool,
        row.endpoint,
        row.args_hash,
        row.credits_estimated,
        row.credits_actual
            .map_or_else(|| "-".to_owned(), |value| value.to_string()),
        row.batch_multiplier,
        row.duration_ms,
        row.status,
        row.response_bytes,
        row.tokens,
        row.tokens_saved,
        row.cache_hit,
    );
    let mut hasher = Sha256::new();
    hasher.update(payload.as_bytes());
    hex::encode(hasher.finalize())
}

fn unix_seconds() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|value| value.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_path(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("ownserp-ledger-{}-{label}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        dir.join("calls.jsonl")
    }

    fn record() -> CallRecord {
        CallRecord {
            tool: "serper_search".to_owned(),
            endpoint: "/search".to_owned(),
            args_hash: "0".repeat(64),
            credits_estimated: 1,
            credits_actual: None,
            batch_multiplier: 1,
            duration_ms: 1,
            status: "ok".to_owned(),
            response_bytes: 10,
            tokens: 2,
            tokens_saved: 1,
            cache_hit: false,
        }
    }

    #[test]
    fn a_second_handle_is_refused_while_the_first_is_open() {
        let path = temp_path("lock");
        let _ = std::fs::remove_file(&path);
        let first = Ledger::open(&path).expect("first handle");
        assert!(
            Ledger::open(&path).is_err(),
            "the lock must refuse a second handle"
        );
        drop(first);
        assert!(Ledger::open(&path).is_ok(), "the lock must release on drop");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_failed_append_blocks_further_reservations() {
        let path = temp_path("unhealthy");
        let _ = std::fs::remove_file(&path);
        let ledger = Ledger::open(&path).expect("ledger");
        let mut broken = record();
        broken.tool = "a".repeat(MAX_LINE_BYTES + 1);
        assert!(ledger.append(broken).is_err());
        assert!(
            ledger.reserve(1, 100).is_err(),
            "a ledger that cannot record must refuse a new reservation"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_reservation_holds_the_budget_until_it_drops() {
        let path = temp_path("reserve");
        let _ = std::fs::remove_file(&path);
        let ledger = Ledger::open(&path).expect("ledger");
        let held = ledger.reserve(6, 10).expect("first reservation");
        assert!(
            ledger.reserve(6, 10).is_err(),
            "6 plus 6 passes a budget of 10"
        );
        drop(held);
        assert!(ledger.reserve(6, 10).is_ok(), "the budget frees on drop");
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn an_oversized_line_is_refused() {
        let path = temp_path("longline");
        let _ = std::fs::remove_file(&path);
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .expect("open");
        file.write_all(&vec![b'a'; MAX_LINE_BYTES + 1])
            .expect("write");
        file.write_all(b"\n").expect("newline");
        drop(file);
        assert!(
            Ledger::open(&path).is_err(),
            "an oversized line must be refused"
        );
        let _ = std::fs::remove_file(&path);
    }

    #[cfg(unix)]
    #[test]
    fn the_ledger_is_owner_only() {
        use std::os::unix::fs::PermissionsExt;
        let root =
            std::env::temp_dir().join(format!("ownserp-ledger-perms-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let path = root.join("ledger").join("calls.jsonl");
        let ledger = Ledger::open(&path).expect("ledger");
        let file_mode = std::fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
        let dir_mode = std::fs::metadata(path.parent().expect("parent"))
            .expect("stat")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(file_mode, 0o600);
        assert_eq!(dir_mode, 0o700);
        drop(ledger);
        let _ = std::fs::remove_file(&path);
    }

    #[cfg(unix)]
    #[test]
    fn a_symlinked_ledger_file_is_refused() {
        use std::os::unix::fs::symlink;
        let dir = std::env::temp_dir().join(format!("ownserp-ledger-link-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");
        let target = dir.join("target");
        std::fs::write(&target, b"x").expect("target");
        let link = dir.join("calls.jsonl");
        symlink(&target, &link).expect("link");
        assert!(
            Ledger::open(&link).is_err(),
            "a symbolic link must be refused"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_unterminated_valid_final_row_is_not_chained() {
        let path = temp_path("unterminated");
        let _ = std::fs::remove_file(&path);
        {
            let ledger = Ledger::open(&path).expect("ledger");
            ledger.append(record()).expect("first");
            ledger.append(record()).expect("second");
        }
        let bytes = std::fs::read(&path).expect("read");
        let trimmed = bytes.strip_suffix(b"\n").expect("newline");
        std::fs::write(&path, trimmed).expect("drop the final newline");

        let ledger = Ledger::open(&path).expect("reopen past an unterminated row");
        assert_eq!(ledger.totals().expect("totals").calls, 1);
        ledger.append(record()).expect("append after repair");
        drop(ledger);

        assert_eq!(verify(&path).expect("chain still valid"), 2);
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn a_partial_final_line_does_not_block_reopening() {
        let path = temp_path("partial");
        let _ = std::fs::remove_file(&path);
        {
            let ledger = Ledger::open(&path).expect("ledger");
            ledger.append(record()).expect("append");
        }
        let mut file = OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("append open");
        file.write_all(b"{\"seq\":2,\"ts_unix\":")
            .expect("partial write");
        drop(file);

        let ledger = Ledger::open(&path).expect("reopen past a partial tail");
        assert_eq!(ledger.totals().expect("totals").calls, 1);
        ledger.append(record()).expect("append after repair");
        drop(ledger);

        let ledger = Ledger::open(&path).expect("second reopen after a repaired tail");
        assert_eq!(ledger.totals().expect("totals").calls, 2);
        assert_eq!(verify(&path).expect("chain still valid"), 2);
        let _ = std::fs::remove_file(&path);
    }
}
