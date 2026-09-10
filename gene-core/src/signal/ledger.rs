use crate::signal::types::SignalSnapshot;
use anyhow::Result;
use sled::Db;
use std::path::Path;

/// Append-only persistent log of signal snapshots.
/// Every tick writes a snapshot here. This is the raw memory of the system.
///
/// QUOTA: enforcement is by BYTES ON DISK, not by entry count.
///
/// It was previously by entry count, derived as `disk_quota_mb * 1MiB / 256` —
/// an assumed 256 bytes per snapshot. Snapshots are nowhere near that size and
/// vary by orders of magnitude: measured on the live instance, sled externalises
/// large values into `ledger/blobs/`, where the mean file is ~182 KB. The result
/// was a flag reading "512 MB" that permitted a 36.9 GiB `db` plus 9.5 GiB of
/// blobs. A byte quota cannot be enforced through a guessed entry size, so this
/// asks sled what it is actually using.
pub struct SignalLedger {
    db: Db,
    /// Track total entries — retained for observability, no longer the quota basis.
    entry_count: u64,
    /// Max bytes on disk before compaction drops the oldest entries.
    quota_bytes: u64,
    /// Entries removed per compaction pass.
    compact_batch: u64,
    /// Ticks since the last size check; `size_on_disk` walks files, so it is not
    /// called on every append.
    since_size_check: u64,
}

/// How often to ask sled for its on-disk size. At ~19 appends/sec this is about
/// once a minute — frequent enough to bound growth, rare enough not to stat the
/// directory tree on every tick.
const SIZE_CHECK_INTERVAL: u64 = 1024;

impl SignalLedger {
    /// `quota_bytes` — target ceiling for the ledger's on-disk footprint.
    /// `compact_batch` — how many of the oldest entries to drop per pass.
    pub fn open(path: &Path, quota_bytes: u64, compact_batch: u64) -> Result<Self> {
        let db = sled::open(path)?;
        let entry_count = db.len() as u64;
        let on_disk = db.size_on_disk().unwrap_or(0);
        tracing::info!(
            "ledger opened: {} entries, {:.2} GiB on disk, quota {:.2} GiB",
            entry_count,
            on_disk as f64 / 1_073_741_824.0,
            quota_bytes as f64 / 1_073_741_824.0,
        );
        Ok(Self {
            db,
            entry_count,
            quota_bytes,
            compact_batch: compact_batch.max(1),
            since_size_check: 0,
        })
    }

    /// Bytes sled reports for this database.
    ///
    /// NOTE: whether this includes externalised blob files is not documented
    /// clearly, so `compact` logs it next to the entry count. If the reported
    /// size diverges from the directory's real size, that will be visible in the
    /// logs rather than silently wrong — which is how the old entry-count quota
    /// stayed wrong for months.
    pub fn size_on_disk(&self) -> u64 {
        self.db.size_on_disk().unwrap_or(0)
    }

    /// Append a snapshot. Returns the tick key used.
    pub fn append(&mut self, snapshot: &SignalSnapshot) -> Result<()> {
        let key = snapshot.tick.to_be_bytes();
        let value = bincode::serialize(snapshot)?;
        self.db.insert(key, value)?;
        self.entry_count += 1;
        self.since_size_check += 1;

        if self.since_size_check >= SIZE_CHECK_INTERVAL {
            self.since_size_check = 0;
            let on_disk = self.size_on_disk();
            if on_disk > self.quota_bytes {
                self.compact(self.compact_batch, on_disk)?;
            }
        }

        Ok(())
    }

    /// Read snapshots in a tick range [from, to].
    pub fn range(&self, from: u64, to: u64) -> Result<Vec<SignalSnapshot>> {
        let from_key = from.to_be_bytes();
        let to_key = to.to_be_bytes();
        let mut out = Vec::new();
        for item in self.db.range(from_key..=to_key) {
            let (_, v) = item?;
            let snap: SignalSnapshot = bincode::deserialize(&v)?;
            out.push(snap);
        }
        Ok(out)
    }

    /// Read the last N snapshots.
    pub fn tail(&self, n: usize) -> Result<Vec<SignalSnapshot>> {
        let mut out: Vec<SignalSnapshot> = Vec::with_capacity(n);
        for item in self.db.iter().rev().take(n) {
            let (_, v) = item?;
            let snap: SignalSnapshot = bincode::deserialize(&v)?;
            out.push(snap);
        }
        out.reverse();
        Ok(out)
    }

    pub fn len(&self) -> u64 {
        self.entry_count
    }

    /// Drop oldest `n` entries to stay under the byte quota.
    fn compact(&mut self, n: u64, on_disk_before: u64) -> Result<()> {
        let mut removed = 0u64;
        for item in self.db.iter() {
            if removed >= n {
                break;
            }
            let (k, _) = item?;
            self.db.remove(k)?;
            removed += 1;
        }
        self.entry_count = self.entry_count.saturating_sub(removed);
        let after = self.size_on_disk();
        tracing::info!(
            "ledger compacted: removed {} entries ({} remain); size_on_disk {:.2} -> {:.2} GiB \
             (quota {:.2} GiB)",
            removed,
            self.entry_count,
            on_disk_before as f64 / 1_073_741_824.0,
            after as f64 / 1_073_741_824.0,
            self.quota_bytes as f64 / 1_073_741_824.0,
        );
        if after >= on_disk_before {
            // sled reuses freed space rather than returning it, so the file may
            // not shrink. Say so plainly instead of letting it look like the
            // compaction failed.
            tracing::info!(
                "ledger: on-disk size did not fall after compaction — sled reuses \
                 freed space internally; the file stays at its high-water mark"
            );
        }
        Ok(())
    }

    pub fn flush(&self) -> Result<()> {
        self.db.flush()?;
        Ok(())
    }
}
