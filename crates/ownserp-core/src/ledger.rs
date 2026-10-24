use std::collections::{BTreeMap, VecDeque};
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{BufReader, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::error::{Error, Result};

const MAX_LINE_BYTES: usize = 1024 * 1024;
const GENESIS: &str = "genesis";
const LOCK_WAIT_STEP: Duration = Duration::from_millis(10);
const LOCK_WAIT_LIMIT: Duration = Duration::from_secs(5);
const MAX_RELOADS: u8 = 8;
const QUEUE_CAPACITY: usize = 256;

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

struct Cursor {
    offset: u64,
    seq: u64,
    last_hash: String,
    totals: Totals,
}

impl Cursor {
    fn genesis() -> Self {
        Self {
            offset: 0,
            seq: 0,
            last_hash: String::from(GENESIS),
            totals: Totals::default(),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CatchUp {
    Applied,
    Replaced,
}

struct LedgerState {
    path: PathBuf,
    file: File,
    cursor: Cursor,
    lock_wait: Duration,
}

struct Queued {
    record: CallRecord,
    observed_unix: u64,
}

#[derive(Clone)]
pub struct Ledger {
    state: Arc<Mutex<LedgerState>>,
    snapshot: Arc<Mutex<Totals>>,
    pending: Arc<AtomicU64>,
    queue: Arc<Mutex<VecDeque<Queued>>>,
    lost: Arc<AtomicU64>,
    healthy: Arc<AtomicBool>,
}

impl Drop for Ledger {
    fn drop(&mut self) {
        if Arc::strong_count(&self.state) != 1 {
            return;
        }
        let batch = self.take_queued();
        if batch.is_empty() {
            return;
        }
        if let Ok(mut state) = self.state.lock() {
            let _ = state.catch_up(true);
            let _ = state.write_batch(batch);
        }
    }
}

pub struct Reservation {
    ledger: Ledger,
    credits: u32,
    settled: AtomicBool,
}

impl Reservation {
    pub fn credits(&self) -> u32 {
        self.credits
    }

    pub fn disarm(&self) {
        self.settled.store(true, Ordering::Release);
    }
}

impl Drop for Reservation {
    fn drop(&mut self) {
        if !self.settled.load(Ordering::Acquire) {
            self.ledger.settle(self.credits);
        }
    }
}

impl Ledger {
    pub fn open(path: impl Into<PathBuf>) -> Result<Self> {
        Self::open_within(path, LOCK_WAIT_LIMIT)
    }

    fn open_within(path: impl Into<PathBuf>, lock_wait: Duration) -> Result<Self> {
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
        ensure_regular_file(&path)?;
        let file = open_ledger_file(&path)?;
        ensure_same_file(&path, &file)?;
        let locked = LockedFile::exclusive(&file, &path, lock_wait)?;
        sync_parent_dir(&path);
        let cursor = load_chain(&file, &path)?;
        let file_len = file.metadata()?.len();
        if file_len > cursor.offset {
            if cursor.offset == 0 {
                return Err(Error::Ledger(format!(
                    "{} holds content that is not a ledger; point the ledger at a new file",
                    file_label(&path)
                )));
            }
            file.set_len(cursor.offset)?;
            tracing::warn!(
                path = %path.display(),
                removed = file_len.saturating_sub(cursor.offset),
                "truncated a partial final ledger line before appending"
            );
        }
        lock_down_file(&path, &file)?;
        drop(locked);
        let published = cursor.totals.clone();
        Ok(Self {
            state: Arc::new(Mutex::new(LedgerState {
                path,
                file,
                cursor,
                lock_wait,
            })),
            snapshot: Arc::new(Mutex::new(published)),
            pending: Arc::new(AtomicU64::new(0)),
            queue: Arc::new(Mutex::new(VecDeque::new())),
            lost: Arc::new(AtomicU64::new(0)),
            healthy: Arc::new(AtomicBool::new(true)),
        })
    }

    pub fn append(&self, record: CallRecord) -> Result<LedgerRow> {
        self.append_reserved(record, 0)
    }

    pub fn append_reserved(&self, record: CallRecord, reserved: u32) -> Result<LedgerRow> {
        let outcome = self.append_inner(record, reserved);
        if let Err(error) = &outcome
            && !matches!(error, Error::LedgerBusy(_))
        {
            self.healthy.store(false, Ordering::Relaxed);
        }
        outcome
    }

    fn append_inner(&self, record: CallRecord, reserved: u32) -> Result<LedgerRow> {
        let mut state = self.lock()?;
        let mut reloads = 0u8;
        loop {
            if !file_matches(&state.path, &state.file)? {
                state.reload(&mut reloads)?;
                self.publish(&state.cursor.totals);
                continue;
            }
            let handle = clone_handle(&state.file)?;
            let locked = match LockedFile::exclusive(&handle, &state.path, state.lock_wait) {
                Ok(locked) => locked,
                Err(error) => {
                    drop(state);
                    if matches!(error, Error::LedgerBusy(_)) {
                        self.enqueue(record, unix_seconds());
                    }
                    return Err(error);
                }
            };
            if state.catch_up(true)? == CatchUp::Replaced {
                drop(locked);
                continue;
            }
            let mut batch = self.take_queued();
            batch.push(Queued {
                record,
                observed_unix: unix_seconds(),
            });
            let rows = state.write_batch(batch);
            if rows.is_ok() {
                self.publish(&state.cursor.totals);
                if reserved > 0 {
                    self.settle(reserved);
                }
            }
            let mut rows = rows?;
            return rows
                .pop()
                .ok_or_else(|| Error::Ledger("no ledger row was written".into()));
        }
    }

    pub fn reserve(&self, credits: u32, budget: u64) -> Result<Reservation> {
        if !self.healthy.load(Ordering::Relaxed) {
            return Err(Error::Ledger(
                "the ledger could not record the last call; restore the ledger directory and restart".into(),
            ));
        }
        let mut state = self.lock()?;
        let mut reloads = 0u8;
        loop {
            let handle = clone_handle(&state.file)?;
            let locked = LockedFile::shared(&handle, &state.path, state.lock_wait)?;
            if state.catch_up(false)? == CatchUp::Replaced {
                drop(locked);
                state.reload(&mut reloads)?;
                self.publish(&state.cursor.totals);
                continue;
            }
            self.publish(&state.cursor.totals);
            let spent = state
                .cursor
                .totals
                .credits
                .saturating_add(self.queued_credits());
            let mut held = self.pending.load(Ordering::Acquire);
            let claimed = loop {
                if spent
                    .saturating_add(held)
                    .saturating_add(u64::from(credits))
                    > budget
                {
                    break Err(held);
                }
                match self.pending.compare_exchange_weak(
                    held,
                    held.saturating_add(u64::from(credits)),
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => break Ok(()),
                    Err(current) => held = current,
                }
            };
            return match claimed {
                Ok(_) => Ok(Reservation {
                    ledger: self.clone(),
                    credits,
                    settled: AtomicBool::new(false),
                }),
                Err(held) => Err(Error::BudgetRefused(format!(
                    "the ledger's budget of {budget} credits would be passed: {credits} more on top of {} already committed",
                    spent.saturating_add(held)
                ))),
            };
        }
    }

    fn enqueue(&self, record: CallRecord, observed_unix: u64) {
        let mut queue = match self.queue.lock() {
            Ok(queue) => queue,
            Err(poisoned) => poisoned.into_inner(),
        };
        if queue.len() >= QUEUE_CAPACITY {
            let _ = queue.pop_front();
            let lost = self.lost.fetch_add(1, Ordering::Relaxed).saturating_add(1);
            tracing::error!(
                lost,
                "the ledger queue is full; the oldest waiting row was dropped"
            );
        }
        queue.push_back(Queued {
            record,
            observed_unix,
        });
    }

    fn take_queued(&self) -> Vec<Queued> {
        let mut queue = match self.queue.lock() {
            Ok(queue) => queue,
            Err(poisoned) => poisoned.into_inner(),
        };
        queue.drain(..).collect()
    }

    fn queued_credits(&self) -> u64 {
        let queue = match self.queue.lock() {
            Ok(queue) => queue,
            Err(poisoned) => poisoned.into_inner(),
        };
        queue
            .iter()
            .map(|queued| u64::from(queued.record.credits_estimated))
            .fold(0u64, u64::saturating_add)
    }

    fn settle(&self, credits: u32) {
        let delta = u64::from(credits);
        let mut held = self.pending.load(Ordering::Acquire);
        loop {
            let next = held.saturating_sub(delta);
            match self.pending.compare_exchange_weak(
                held,
                next,
                Ordering::AcqRel,
                Ordering::Acquire,
            ) {
                Ok(_) => return,
                Err(current) => held = current,
            }
        }
    }

    pub fn totals(&self) -> Result<Totals> {
        let mut state = self.lock()?;
        let mut reloads = 0u8;
        loop {
            let handle = clone_handle(&state.file)?;
            let locked = LockedFile::shared(&handle, &state.path, state.lock_wait)?;
            if state.catch_up(false)? == CatchUp::Replaced {
                drop(locked);
                state.reload(&mut reloads)?;
                self.publish(&state.cursor.totals);
                continue;
            }
            self.publish(&state.cursor.totals);
            return Ok(state.cursor.totals.clone());
        }
    }

    pub fn snapshot(&self) -> Totals {
        let published = match self.snapshot.lock() {
            Ok(published) => published,
            Err(poisoned) => poisoned.into_inner(),
        };
        published.clone()
    }

    fn publish(&self, totals: &Totals) {
        let mut published = match self.snapshot.lock() {
            Ok(published) => published,
            Err(poisoned) => poisoned.into_inner(),
        };
        *published = totals.clone();
    }

    pub fn pending(&self) -> Result<u64> {
        Ok(self.pending.load(Ordering::Acquire))
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

impl LedgerState {
    fn catch_up(&mut self, repair: bool) -> Result<CatchUp> {
        if !file_matches(&self.path, &self.file)? {
            return Ok(CatchUp::Replaced);
        }
        let LedgerState {
            path, file, cursor, ..
        } = self;
        let path: &Path = path;
        let file: &File = file;
        let file_len = file.metadata()?.len();
        if file_len < cursor.offset {
            return Err(Error::Ledger(format!(
                "the ledger at {} is shorter than the part this process already read; restore it or start a fresh one",
                path.display()
            )));
        }
        if file_len == cursor.offset {
            return Ok(CatchUp::Applied);
        }
        let mut previous = cursor.last_hash.clone();
        let seq = &mut cursor.seq;
        let totals = &mut cursor.totals;
        let (_, valid_end) = read_rows(file, path, cursor.offset, false, |row| {
            let expected = chain_hash(&previous, row);
            if row.hash != expected || row.prev != previous {
                return Err(Error::Ledger(format!(
                    "hash chain broken at sequence {} in {}",
                    row.seq,
                    path.display()
                )));
            }
            previous = row.hash.clone();
            *seq = row.seq;
            accumulate(totals, row);
            Ok(())
        })?;
        cursor.offset = valid_end;
        cursor.last_hash = previous;
        if repair && file_len > valid_end {
            file.set_len(valid_end)?;
            tracing::warn!(
                path = %path.display(),
                removed = file_len.saturating_sub(valid_end),
                "truncated a partial final ledger line before appending"
            );
        }
        Ok(CatchUp::Applied)
    }

    fn reload(&mut self, reloads: &mut u8) -> Result<()> {
        self.adopt_current_file()?;
        *reloads = reloads.saturating_add(1);
        if *reloads > MAX_RELOADS {
            return Err(Error::Ledger(format!(
                "the ledger at {} keeps being replaced; stop whatever is rotating it",
                self.path.display()
            )));
        }
        Ok(())
    }

    fn adopt_current_file(&mut self) -> Result<()> {
        ensure_regular_file(&self.path)?;
        let file = open_ledger_file(&self.path)?;
        ensure_same_file(&self.path, &file)?;
        let locked = LockedFile::exclusive(&file, &self.path, self.lock_wait)?;
        sync_parent_dir(&self.path);
        let cursor = load_chain(&file, &self.path)?;
        let file_len = file.metadata()?.len();
        if file_len > cursor.offset {
            if cursor.offset == 0 {
                return Err(Error::Ledger(format!(
                    "{} holds content that is not a ledger; point the ledger at a new file",
                    file_label(&self.path)
                )));
            }
            file.set_len(cursor.offset)?;
        }
        lock_down_file(&self.path, &file)?;
        drop(locked);
        self.file = file;
        self.cursor = cursor;
        Ok(())
    }

    fn write_batch(&mut self, batch: Vec<Queued>) -> Result<Vec<LedgerRow>> {
        let mut rows = Vec::with_capacity(batch.len());
        let mut lines = String::new();
        let mut seq = self.cursor.seq;
        let mut previous = self.cursor.last_hash.clone();
        for queued in batch {
            let record = queued.record;
            let row = LedgerRow {
                seq: seq.saturating_add(1),
                ts_unix: queued.observed_unix,
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
                prev: previous.clone(),
                hash: String::new(),
            };
            let hash = chain_hash(&previous, &row);
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
            seq = written.seq;
            previous = written.hash.clone();
            lines.push_str(&line);
            rows.push(written);
        }
        let mut writer = &self.file;
        writer.write_all(lines.as_bytes())?;
        for row in &rows {
            accumulate(&mut self.cursor.totals, row);
        }
        self.cursor.seq = seq;
        self.cursor.last_hash = previous;
        self.cursor.offset = self.cursor.offset.saturating_add(lines.len() as u64);
        writer.sync_data().map_err(|error| Error::Io {
            context: "syncing the ledger after the row was written",
            source: error,
        })?;
        Ok(rows)
    }
}

pub fn verify(path: &Path) -> Result<u64> {
    ensure_regular_file(path)?;
    let file = File::open(path)?;
    ensure_same_file(path, &file)?;
    let _locked = LockedFile::shared(&file, path, LOCK_WAIT_LIMIT)?;
    count_chain(&file, path)
}

pub fn rotate(path: &Path) -> Result<(u64, PathBuf)> {
    if !path.exists() {
        return Err(Error::Ledger(format!(
            "no ledger at {} to rotate",
            path.display()
        )));
    }
    ensure_regular_file(path)?;
    let file = OpenOptions::new().read(true).append(true).open(path)?;
    ensure_same_file(path, &file)?;
    let _locked = LockedFile::exclusive(&file, path, LOCK_WAIT_LIMIT)?;
    ensure_same_file(path, &file)?;
    let rows = count_chain(&file, path)?;
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
    sync_parent_dir(path);
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

fn read_rows<F>(file: &File, path: &Path, start: u64, strict: bool, visit: F) -> Result<(u64, u64)>
where
    F: FnMut(&LedgerRow) -> Result<()>,
{
    let mut seeker = file;
    seeker.seek(SeekFrom::Start(start))?;
    let mut reader = BufReader::new(file);
    let (rows, valid_len) = walk_reader(&mut reader, path, strict, visit)?;
    Ok((rows, start.saturating_add(valid_len)))
}

fn load_chain(file: &File, path: &Path) -> Result<Cursor> {
    let mut cursor = Cursor::genesis();
    let mut previous = String::from(GENESIS);
    let (_, valid_end) = read_rows(file, path, 0, false, |row| {
        let expected = chain_hash(&previous, row);
        if row.hash != expected || row.prev != previous {
            return Err(Error::Ledger(format!(
                "hash chain broken at sequence {} in {}",
                row.seq,
                path.display()
            )));
        }
        previous = row.hash.clone();
        cursor.seq = row.seq;
        cursor.last_hash = row.hash.clone();
        accumulate(&mut cursor.totals, row);
        Ok(())
    })?;
    cursor.offset = valid_end;
    Ok(cursor)
}

fn count_chain(file: &File, path: &Path) -> Result<u64> {
    let mut previous = String::from(GENESIS);
    let mut expected_seq = 1u64;
    let (rows, _) = read_rows(file, path, 0, true, |row| {
        if row.seq != expected_seq {
            return Err(Error::Ledger(format!(
                "sequence {} follows {}, which leaves a gap",
                row.seq,
                expected_seq.saturating_sub(1)
            )));
        }
        expected_seq = expected_seq.saturating_add(1);
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
        Ok(())
    })?;
    Ok(rows)
}

fn walk_reader<R, F>(reader: &mut R, path: &Path, strict: bool, mut visit: F) -> Result<(u64, u64)>
where
    R: std::io::BufRead,
    F: FnMut(&LedgerRow) -> Result<()>,
{
    let mut buffer = Vec::new();
    let mut consumed = 0u64;
    let mut rows = 0u64;
    let mut valid_len = 0u64;
    loop {
        buffer.clear();
        let Some(line) = read_line_capped(reader)? else {
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
                rows = rows.saturating_add(1);
                valid_len = consumed;
            }
            Ok(_) => {
                if strict {
                    return Err(Error::Ledger(format!(
                        "{} ends with an unterminated line",
                        file_label(path)
                    )));
                }
                tracing::warn!(
                    path = %path.display(),
                    "ignoring an unterminated final ledger line left by an interrupted write"
                );
                break;
            }
            Err(error) => {
                if !terminated {
                    if strict {
                        return Err(Error::Ledger(format!(
                            "{} ends with a partial line",
                            file_label(path)
                        )));
                    }
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
    Ok((rows, valid_len))
}

fn sync_parent_dir(path: &Path) {
    #[cfg(unix)]
    {
        if let Some(parent) = path.parent()
            && let Ok(handle) = File::open(parent)
            && let Err(error) = handle.sync_all()
        {
            tracing::warn!(%error, path = %parent.display(), "could not sync the ledger directory");
        }
    }
    #[cfg(not(unix))]
    let _ = path;
}

fn file_label(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("the ledger")
        .to_owned()
}

#[cfg(all(unix, target_os = "linux"))]
const NO_FOLLOW: i32 = 0o400000;
#[cfg(all(unix, not(target_os = "linux")))]
const NO_FOLLOW: i32 = 0x0100;

fn open_ledger_file(path: &Path) -> Result<File> {
    let mut options = OpenOptions::new();
    options.create(true).read(true).append(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
        // The open must not follow a symlink planted between the check and this call.
        options.custom_flags(NO_FOLLOW);
    }
    Ok(options.open(path)?)
}

fn clone_handle(file: &File) -> Result<File> {
    file.try_clone().map_err(|error| Error::Io {
        context: "cloning the ledger handle",
        source: error,
    })
}

struct LockedFile<'a> {
    file: &'a File,
}

impl<'a> LockedFile<'a> {
    fn exclusive(file: &'a File, path: &Path, limit: Duration) -> Result<Self> {
        wait_for_lock(file, path, limit, true)?;
        Ok(Self { file })
    }

    fn shared(file: &'a File, path: &Path, limit: Duration) -> Result<Self> {
        wait_for_lock(file, path, limit, false)?;
        Ok(Self { file })
    }
}

impl Drop for LockedFile<'_> {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

fn wait_for_lock(file: &File, path: &Path, limit: Duration, exclusive: bool) -> Result<()> {
    let deadline = Instant::now() + limit;
    loop {
        let outcome = if exclusive {
            file.try_lock()
        } else {
            file.try_lock_shared()
        };
        match outcome {
            Ok(()) => return Ok(()),
            Err(TryLockError::WouldBlock) => {}
            Err(error) => {
                return Err(Error::Io {
                    context: "locking the ledger",
                    source: error.into(),
                });
            }
        }
        if Instant::now() >= deadline {
            return Err(Error::LedgerBusy(format!(
                "{} is held by another process",
                path.file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or("the ledger")
            )));
        }
        std::thread::sleep(LOCK_WAIT_STEP);
    }
}

fn ensure_regular_file(path: &Path) -> Result<()> {
    match std::fs::symlink_metadata(path) {
        Ok(metadata) => {
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
            Ok(())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(Error::Ledger(format!(
            "cannot inspect {}: {error}",
            path.display()
        ))),
    }
}

#[cfg(unix)]
fn file_matches(path: &Path, file: &File) -> Result<bool> {
    use std::os::unix::fs::MetadataExt;

    let opened = file
        .metadata()
        .map_err(|error| Error::Ledger(format!("cannot stat the open ledger: {error}")))?;
    match std::fs::symlink_metadata(path) {
        Ok(on_disk) => {
            if on_disk.file_type().is_symlink() || !on_disk.is_file() {
                return Err(Error::Ledger(format!(
                    "{} is not a regular file; make the ledger directory private and retry",
                    path.display()
                )));
            }
            Ok(opened.dev() == on_disk.dev() && opened.ino() == on_disk.ino())
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(Error::Ledger(format!(
            "cannot re-check {}: {error}",
            path.display()
        ))),
    }
}

#[cfg(not(unix))]
fn file_matches(_path: &Path, _file: &File) -> Result<bool> {
    Ok(true)
}

fn ensure_same_file(path: &Path, file: &File) -> Result<()> {
    if file_matches(path, file)? {
        Ok(())
    } else {
        Err(Error::Ledger(format!(
            "{} changed while it was opened; make the ledger directory private and retry",
            path.display()
        )))
    }
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
fn lock_down_file(path: &Path, file: &File) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    file.set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(|error| {
            Error::Ledger(format!(
                "cannot restrict {} to its owner: {error}",
                path.display()
            ))
        })
}

#[cfg(not(unix))]
fn lock_down_file(_path: &Path, _file: &File) -> Result<()> {
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

    fn clean(path: &Path) {
        let _ = std::fs::remove_file(path);
        if let Some(parent) = path.parent()
            && parent != std::env::temp_dir()
        {
            let _ = std::fs::remove_dir(parent);
        }
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

    fn record_cost(credits: u32) -> CallRecord {
        let mut priced = record();
        priced.credits_estimated = credits;
        priced
    }

    #[test]
    fn two_handles_open_on_one_path() {
        let path = temp_path("shared-open");
        let _ = std::fs::remove_file(&path);
        let first = Ledger::open(&path).expect("first handle");
        let second = Ledger::open(&path).expect("second handle");
        assert_eq!(second.totals().expect("totals").calls, 0);
        drop((first, second));
        clean(&path);
    }

    #[test]
    fn interleaved_appends_give_a_contiguous_chain() {
        let path = temp_path("interleaved");
        let _ = std::fs::remove_file(&path);
        let first = Ledger::open(&path).expect("first handle");
        let second = Ledger::open(&path).expect("second handle");
        let one = first.append(record()).expect("first row");
        let two = second.append(record()).expect("second row");
        let three = first.append(record()).expect("third row");
        let four = second.append(record()).expect("fourth row");
        assert_eq!((one.seq, two.seq, three.seq, four.seq), (1, 2, 3, 4));
        assert_eq!(two.prev, one.hash);
        assert_eq!(three.prev, two.hash);
        assert_eq!(four.prev, three.hash);
        drop((first, second));
        assert_eq!(verify(&path).expect("chain verifies"), 4);
        clean(&path);
    }

    #[test]
    fn totals_include_rows_written_by_another_handle() {
        let path = temp_path("shared-totals");
        let _ = std::fs::remove_file(&path);
        let first = Ledger::open(&path).expect("first handle");
        let second = Ledger::open(&path).expect("second handle");
        first.append(record()).expect("first row");
        second.append(record()).expect("second row");
        second.append(record()).expect("third row");
        assert_eq!(first.totals().expect("first totals").calls, 3);
        assert_eq!(second.totals().expect("second totals").calls, 3);
        drop((first, second));
        clean(&path);
    }

    #[test]
    fn a_budget_spent_through_one_handle_refuses_the_other() {
        let path = temp_path("shared-budget");
        let _ = std::fs::remove_file(&path);
        let first = Ledger::open(&path).expect("first handle");
        let second = Ledger::open(&path).expect("second handle");
        let held = first.reserve(8, 10).expect("reservation");
        first.append(record_cost(8)).expect("append");
        drop(held);
        assert!(
            second.reserve(3, 10).is_err(),
            "8 already written and 3 more passes a budget of 10"
        );
        assert!(second.reserve(2, 10).is_ok(), "2 still fits a budget of 10");
        drop((first, second));
        clean(&path);
    }

    #[test]
    fn eight_threads_append_one_chain() {
        let path = temp_path("threads");
        let _ = std::fs::remove_file(&path);
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let path = path.clone();
                std::thread::spawn(move || {
                    let ledger = Ledger::open(&path).expect("handle");
                    for _ in 0..50 {
                        ledger.append(record()).expect("append");
                    }
                    ledger
                })
            })
            .collect();
        let handles: Vec<Ledger> = workers
            .into_iter()
            .map(|worker| worker.join().expect("join"))
            .collect();
        for ledger in &handles {
            assert_eq!(ledger.totals().expect("totals").calls, 400);
        }
        assert_eq!(verify(&path).expect("chain verifies"), 400);
        let text = std::fs::read_to_string(&path).expect("read the ledger");
        let mut seqs: Vec<u64> = text
            .lines()
            .map(|line| serde_json::from_str::<LedgerRow>(line).expect("row").seq)
            .collect();
        seqs.sort_unstable();
        assert_eq!(seqs, (1..=400).collect::<Vec<u64>>());
        clean(&path);
    }

    #[test]
    fn rotate_while_a_handle_is_open_starts_a_new_chain() {
        let path = temp_path("rotate-open");
        let _ = std::fs::remove_file(&path);
        let ledger = Ledger::open(&path).expect("ledger");
        ledger.append(record()).expect("first row");
        let (rows, archive) = rotate(&path).expect("rotate");
        assert_eq!(rows, 1);
        let appended = ledger.append(record()).expect("append after rotate");
        assert_eq!(appended.seq, 1);
        assert_eq!(appended.prev, GENESIS);
        assert_eq!(ledger.totals().expect("totals").calls, 1);
        assert_eq!(verify(&archive).expect("archive verifies"), 1);
        assert_eq!(verify(&path).expect("new chain verifies"), 1);
        drop(ledger);
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&archive);
    }

    #[test]
    fn a_replaced_file_is_reported_under_the_lock() {
        let path = temp_path("replaced");
        let _ = std::fs::remove_file(&path);
        let ledger = Ledger::open(&path).expect("ledger");
        ledger.append(record()).expect("first row");
        let replacement = path.with_extension("replacement");
        std::fs::write(&replacement, b"").expect("replacement");
        std::fs::rename(&replacement, &path).expect("swap in a new file");
        let mut state = ledger.lock().expect("state");
        assert_eq!(state.catch_up(true).expect("catch up"), CatchUp::Replaced);
        drop(state);
        drop(ledger);
        clean(&path);
    }

    #[test]
    fn a_snapshot_never_waits_for_the_file_lock() {
        let path = temp_path("snapshot-fast");
        let _ = std::fs::remove_file(&path);
        let ledger = Ledger::open(&path).expect("ledger");
        ledger.append(record()).expect("append");
        let holder = OpenOptions::new()
            .read(true)
            .append(true)
            .open(&path)
            .expect("holder");
        holder.try_lock().expect("exclusive lock");
        let releaser = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(400));
            let _ = holder.unlock();
        });
        let started = Instant::now();
        let totals = ledger.snapshot();
        let elapsed = started.elapsed();
        assert_eq!(totals.calls, 1);
        assert!(
            elapsed < Duration::from_millis(250),
            "the snapshot waited {elapsed:?} for a held file lock"
        );
        let _ = releaser.join();
        drop(ledger);
        clean(&path);
    }

    #[test]
    fn a_snapshot_includes_rows_from_another_handle() {
        let path = temp_path("snapshot-shared");
        let _ = std::fs::remove_file(&path);
        let first = Ledger::open(&path).expect("first");
        let second = Ledger::open(&path).expect("second");
        first.append(record()).expect("first row");
        second.append(record()).expect("second row");
        first.append(record()).expect("third row");
        assert_eq!(first.snapshot().calls, 3);
        assert_eq!(
            first.snapshot().calls,
            first.totals().expect("totals").calls
        );
        drop((first, second));
        clean(&path);
    }

    #[test]
    fn a_snapshot_never_waits_for_the_state_mutex() {
        let path = temp_path("snapshot-mutex");
        let _ = std::fs::remove_file(&path);
        let ledger = Ledger::open(&path).expect("ledger");
        ledger.append(record()).expect("append");
        let holder = OpenOptions::new()
            .read(true)
            .append(true)
            .open(&path)
            .expect("holder");
        holder.try_lock().expect("exclusive lock");
        let writer = {
            let ledger = ledger.clone();
            std::thread::spawn(move || ledger.append(record()))
        };
        std::thread::sleep(Duration::from_millis(250));
        let started = Instant::now();
        let totals = ledger.snapshot();
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_millis(250),
            "the snapshot waited {elapsed:?} behind a thread that holds the state mutex"
        );
        assert_eq!(totals.calls, 1);
        let _ = holder.unlock();
        assert!(writer.join().expect("join").is_ok());
        drop(ledger);
        clean(&path);
    }

    #[test]
    fn opening_waits_for_an_exclusive_holder() {
        let path = temp_path("wait");
        let _ = std::fs::remove_file(&path);
        Ledger::open(&path).expect("create the file");
        let holder = OpenOptions::new()
            .read(true)
            .append(true)
            .open(&path)
            .expect("holder");
        holder.try_lock().expect("exclusive lock");
        let waiter = {
            let path = path.clone();
            std::thread::spawn(move || Ledger::open(&path).map(|_| ()))
        };
        std::thread::sleep(Duration::from_millis(250));
        holder.unlock().expect("unlock");
        waiter
            .join()
            .expect("join")
            .expect("the open succeeds once the holder releases");
        clean(&path);
    }

    #[test]
    fn a_lock_held_past_the_limit_fails_with_a_plain_message() {
        let path = temp_path("timeout");
        let _ = std::fs::remove_file(&path);
        Ledger::open(&path).expect("create the file");
        let holder = OpenOptions::new()
            .read(true)
            .append(true)
            .open(&path)
            .expect("holder");
        holder.try_lock().expect("exclusive lock");
        let started = Instant::now();
        let outcome = Ledger::open_within(&path, Duration::from_millis(60));
        let elapsed = started.elapsed();
        let message = outcome
            .err()
            .expect("a held lock must not open")
            .to_string();
        assert!(message.contains("held by another process"), "{message}");
        assert!(
            !message.contains('/'),
            "a client-facing message must not carry a path: {message}"
        );
        assert!(
            elapsed >= Duration::from_millis(60),
            "the wait must respect the limit"
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "the wait must not hang: {elapsed:?}"
        );
        holder.unlock().expect("unlock");
        clean(&path);
    }

    #[test]
    fn a_busy_ledger_stays_healthy() {
        let path = temp_path("busy-healthy");
        let _ = std::fs::remove_file(&path);
        let ledger = Ledger::open_within(&path, Duration::from_millis(60)).expect("ledger");
        let holder = OpenOptions::new()
            .read(true)
            .append(true)
            .open(&path)
            .expect("holder");
        holder.try_lock().expect("exclusive lock");
        let outcome = ledger.append(record());
        let message = outcome.expect_err("a held lock must fail").to_string();
        assert!(message.contains("busy"), "{message}");
        let _ = holder.unlock();
        assert!(
            ledger.reserve(1, 100).is_ok(),
            "a busy lock must not latch the ledger as broken"
        );
        clean(&path);
    }

    #[test]
    fn a_rotate_refuses_a_file_replaced_while_it_waited() {
        let path = temp_path("rotate-race");
        let _ = std::fs::remove_file(&path);
        {
            let ledger = Ledger::open(&path).expect("ledger");
            ledger.append(record()).expect("append");
        }
        let holder = OpenOptions::new()
            .read(true)
            .append(true)
            .open(&path)
            .expect("holder");
        holder.try_lock().expect("exclusive lock");
        let rotating = {
            let path = path.clone();
            std::thread::spawn(move || rotate(&path))
        };
        std::thread::sleep(Duration::from_millis(150));
        let replacement = path.with_extension("replacement");
        std::fs::write(&replacement, b"").expect("replacement");
        std::fs::rename(&replacement, &path).expect("swap in a new file");
        let _ = holder.unlock();
        let outcome = rotating.join().expect("join");
        assert!(
            outcome.is_err(),
            "a rotate must not move a file it no longer owns"
        );
        clean(&path);
    }

    #[test]
    fn dropping_a_reservation_never_waits_for_the_file_lock() {
        let path = temp_path("settle-fast");
        let _ = std::fs::remove_file(&path);
        let ledger = Ledger::open(&path).expect("ledger");
        ledger.append(record()).expect("append");
        let reservation = ledger.reserve(1, 100).expect("reservation");
        let holder = OpenOptions::new()
            .read(true)
            .append(true)
            .open(&path)
            .expect("holder");
        holder.try_lock().expect("exclusive lock");
        let writer = {
            let ledger = ledger.clone();
            std::thread::spawn(move || ledger.append(record()))
        };
        std::thread::sleep(Duration::from_millis(250));
        let started = Instant::now();
        drop(reservation);
        let elapsed = started.elapsed();
        assert!(
            elapsed < Duration::from_millis(50),
            "dropping a reservation waited {elapsed:?} behind a held file lock"
        );
        let _ = holder.unlock();
        assert!(writer.join().expect("join").is_ok());
        drop(ledger);
        clean(&path);
    }

    #[test]
    fn an_appended_call_releases_its_share_of_the_budget() {
        let path = temp_path("settle-on-append");
        let _ = std::fs::remove_file(&path);
        let first = Ledger::open(&path).expect("first");
        let second = Ledger::open(&path).expect("second");
        first.append(record_cost(8)).expect("spend eight");
        let reservation = first.reserve(1, 10).expect("reservation");
        let held = reservation.credits();
        first
            .append_reserved(record_cost(1), held)
            .expect("append the reserved call");
        reservation.disarm();
        drop(reservation);
        assert!(
            second.reserve(1, 10).is_ok(),
            "a call that is already in the chain must not still hold a credit"
        );
        drop((first, second));
        clean(&path);
    }

    #[test]
    fn a_foreign_file_is_refused_instead_of_emptied() {
        let path = temp_path("foreign");
        let _ = std::fs::remove_file(&path);
        std::fs::write(&path, b"hello").expect("foreign file");
        assert!(
            Ledger::open(&path).is_err(),
            "a file that is not a ledger must be refused"
        );
        assert_eq!(
            std::fs::metadata(&path).expect("stat").len(),
            5,
            "a foreign file must be left as it was"
        );
        clean(&path);
    }

    #[test]
    fn verify_reports_a_missing_ledger() {
        let path = temp_path("verify-missing");
        let _ = std::fs::remove_file(&path);
        assert!(
            verify(&path).is_err(),
            "a missing ledger must not pass as zero rows"
        );
    }

    #[test]
    fn verify_reports_a_damaged_tail() {
        let path = temp_path("verify-tail");
        let _ = std::fs::remove_file(&path);
        {
            let ledger = Ledger::open(&path).expect("ledger");
            ledger.append(record()).expect("append");
        }
        let mut file = OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("append open");
        file.write_all(b"{\"seq\":2").expect("partial write");
        drop(file);
        assert!(
            verify(&path).is_err(),
            "a partial final line must be reported, not skipped"
        );
        clean(&path);
    }

    #[test]
    fn verify_reports_a_gap_in_the_sequence() {
        let path = temp_path("verify-gap");
        let _ = std::fs::remove_file(&path);
        let mut previous = String::from(GENESIS);
        let mut lines = String::new();
        for seq in [1u64, 3u64] {
            let row = LedgerRow {
                seq,
                ts_unix: 1,
                tool: "serper_search".to_owned(),
                endpoint: "/search".to_owned(),
                args_hash: "0".repeat(64),
                credits_estimated: 1,
                credits_actual: None,
                batch_multiplier: 1,
                duration_ms: 1,
                status: "ok".to_owned(),
                response_bytes: 1,
                tokens: 0,
                tokens_saved: 0,
                cache_hit: false,
                prev: previous.clone(),
                hash: String::new(),
            };
            let hash = chain_hash(&previous, &row);
            let row = LedgerRow { hash, ..row };
            previous = row.hash.clone();
            lines.push_str(&serde_json::to_string(&row).expect("row"));
            lines.push('\n');
        }
        std::fs::write(&path, lines).expect("write");
        assert!(
            verify(&path).is_err(),
            "a gap in the sequence must be refused"
        );
        clean(&path);
    }

    #[test]
    fn a_queued_row_is_written_first_and_keeps_its_time() {
        let path = temp_path("queue-order");
        let _ = std::fs::remove_file(&path);
        let ledger = Ledger::open_within(&path, Duration::from_millis(60)).expect("ledger");
        let holder = OpenOptions::new()
            .read(true)
            .append(true)
            .open(&path)
            .expect("holder");
        holder.try_lock().expect("exclusive lock");
        let queued = ledger.append(record());
        assert!(matches!(queued, Err(Error::LedgerBusy(_))), "{queued:?}");
        let _ = holder.unlock();
        let own = ledger.append(record()).expect("append after the queue");
        assert_eq!(own.seq, 2, "the queued row is written first");
        assert_eq!(verify(&path).expect("chain verifies"), 2);
        let text = std::fs::read_to_string(&path).expect("read");
        let first: LedgerRow =
            serde_json::from_str(text.lines().next().expect("first row")).expect("row");
        assert!(
            first.ts_unix <= own.ts_unix,
            "the queued row keeps the time it was observed"
        );
        drop(ledger);
        clean(&path);
    }

    #[test]
    fn a_queued_row_counts_against_the_budget() {
        let path = temp_path("queue-budget");
        let _ = std::fs::remove_file(&path);
        let ledger = Ledger::open_within(&path, Duration::from_millis(60)).expect("ledger");
        let holder = OpenOptions::new()
            .read(true)
            .append(true)
            .open(&path)
            .expect("holder");
        holder.try_lock().expect("exclusive lock");
        assert!(ledger.append(record()).is_err());
        let _ = holder.unlock();
        assert!(
            ledger.reserve(1, 1).is_err(),
            "a queued credit is still going to be spent"
        );
        drop(ledger);
        clean(&path);
    }

    #[test]
    fn dropping_the_last_handle_flushes_the_queue() {
        let path = temp_path("queue-flush");
        let _ = std::fs::remove_file(&path);
        let ledger = Ledger::open_within(&path, Duration::from_millis(60)).expect("ledger");
        let holder = OpenOptions::new()
            .read(true)
            .append(true)
            .open(&path)
            .expect("holder");
        holder.try_lock().expect("exclusive lock");
        assert!(ledger.append(record()).is_err());
        let _ = holder.unlock();
        drop(ledger);
        assert_eq!(verify(&path).expect("the queue must be flushed on drop"), 1);
        clean(&path);
    }

    #[test]
    fn two_handles_can_reserve_at_once_within_one_ceiling() {
        let path = temp_path("overshoot");
        let _ = std::fs::remove_file(&path);
        let first = Ledger::open(&path).expect("first");
        let second = Ledger::open(&path).expect("second");
        assert!(first.reserve(8, 10).is_ok());
        assert!(
            second.reserve(8, 10).is_ok(),
            "a reservation held in one process is invisible to the other"
        );
        drop((first, second));
        clean(&path);
    }

    #[test]
    fn a_partial_final_line_is_removed_on_open() {
        let path = temp_path("trim");
        let _ = std::fs::remove_file(&path);
        {
            let ledger = Ledger::open(&path).expect("ledger");
            ledger.append(record()).expect("append");
        }
        let complete = std::fs::metadata(&path).expect("stat").len();
        let mut file = OpenOptions::new()
            .append(true)
            .open(&path)
            .expect("append open");
        file.write_all(b"{\"seq\":2").expect("partial write");
        drop(file);
        let ledger = Ledger::open(&path).expect("reopen past a partial tail");
        assert_eq!(ledger.totals().expect("totals").calls, 1);
        assert_eq!(
            std::fs::metadata(&path).expect("stat").len(),
            complete,
            "the partial line must be gone"
        );
        drop(ledger);
        clean(&path);
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
        clean(&path);
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
        clean(&path);
    }

    #[test]
    fn a_directory_is_not_a_ledger() {
        let dir = std::env::temp_dir().join(format!("ownserp-ledger-dir-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("dir");
        assert!(Ledger::open(&dir).is_err());
        assert!(verify(&dir).is_err());
        let _ = std::fs::remove_dir_all(&dir);
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
        clean(&path);
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
        clean(&path);
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
        clean(&path);
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
        clean(&path);
    }
}
