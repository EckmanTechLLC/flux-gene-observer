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
    /// Max bytes on disk before compaction drops the oldest entries.
    quota_bytes: u64,
    /// Entries removed per compaction pass.
    compact_batch: u64,
    /// Ticks since the last size check; `size_on_disk` walks files, so it is not
    /// called on every append.
    since_size_check: u64,
    /// Size at which compaction last failed to reclaim anything.
    ///
    /// sled reuses freed space rather than returning it, so dropping entries
    /// often leaves the on-disk size unchanged. Without this, an over-quota
    /// ledger would compact every check forever — shedding 50,000 entries a
    /// minute and never getting under the limit, draining the whole ledger while
    /// reclaiming nothing. Once compaction is seen to be ineffective, further
    /// attempts are suppressed until the footprint actually grows past this
    /// mark, and the operator is told an offline rebuild is what reclaims space.
    ineffective_above: Option<u64>,
}

/// How often to ask sled for its on-disk size. At ~19 appends/sec this is about
/// once a minute — frequent enough to bound growth, rare enough not to stat the
/// directory tree on every tick.
const SIZE_CHECK_INTERVAL: u64 = 1024;

/// Once compaction is found ineffective, it is not retried until the footprint
/// has grown by at least this much beyond the ineffective mark.
const INEFFECTIVE_RETRY_MARGIN: u64 = 1024 * 1024 * 1024; // 1 GiB

impl SignalLedger {
    /// `quota_bytes` — target ceiling for the ledger's on-disk footprint.
    /// `compact_batch` — how many of the oldest entries to drop per pass.
    ///
    /// Deliberately does NOT count the entries. `db.len()` is O(n) in sled, and
    /// on the live instance walking the 1.72M-entry ledger took 38 minutes, the
    /// whole of which observer-gene spent before its first tick, publishing
    /// nothing. The quota is enforced in bytes and no caller needed the count,
    /// so the scan bought a 38-minute startup for a log line. Do not reintroduce
    /// it: report size, which sled answers without a walk.
    pub fn open(path: &Path, quota_bytes: u64, compact_batch: u64) -> Result<Self> {
        let db = sled::open(path)?;
        let on_disk = db.size_on_disk().unwrap_or(0);
        tracing::info!(
            "ledger opened: {:.2} GiB on disk, quota {:.2} GiB",
            on_disk as f64 / 1_073_741_824.0,
            quota_bytes as f64 / 1_073_741_824.0,
        );
        Ok(Self {
            db,
            quota_bytes,
            compact_batch: compact_batch.max(1),
            since_size_check: 0,
            ineffective_above: None,
        })
    }

    /// Bytes sled reports for this database.
    ///
    /// This DOES include externalised blob files — measured, not assumed. On the
    /// live instance sled reported 46.41 GiB while the directory held 36.87 GiB
    /// of `db` plus 9.54 GiB of `blobs/`, which sum to exactly that. So this is
    /// a sound basis for the byte quota; the old entry-count quota was not, and
    /// stayed wrong for months because nothing ever checked it against reality.
    pub fn size_on_disk(&self) -> u64 {
        self.db.size_on_disk().unwrap_or(0)
    }

    /// Append a snapshot. Returns the tick key used.
    pub fn append(&mut self, snapshot: &SignalSnapshot) -> Result<()> {
        let key = snapshot.tick.to_be_bytes();
        let value = bincode::serialize(snapshot)?;
        self.db.insert(key, value)?;
        self.since_size_check += 1;

        if self.since_size_check >= SIZE_CHECK_INTERVAL {
            self.since_size_check = 0;
            let on_disk = self.size_on_disk();
            let worth_trying = match self.ineffective_above {
                // Compaction already proved unable to reclaim at this size. Only
                // try again once the footprint has genuinely grown past it.
                Some(mark) => on_disk > mark.saturating_add(INEFFECTIVE_RETRY_MARGIN),
                None => true,
            };
            if on_disk > self.quota_bytes && worth_trying {
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
        let after = self.size_on_disk();
        tracing::info!(
            "ledger compacted: removed {} entries; size_on_disk {:.2} -> {:.2} GiB \
             (quota {:.2} GiB)",
            removed,
            on_disk_before as f64 / 1_073_741_824.0,
            after as f64 / 1_073_741_824.0,
            self.quota_bytes as f64 / 1_073_741_824.0,
        );
        if after >= on_disk_before {
            // sled reuses freed space rather than returning it, so dropping
            // entries frequently does not shrink the file. Suppress further
            // attempts rather than draining the ledger for nothing.
            self.ineffective_above = Some(after);
            tracing::warn!(
                "ledger: compaction reclaimed no disk ({:.2} GiB before and after). sled \
                 reuses freed space internally rather than returning it, so the quota \
                 cannot be met by dropping entries. Further compaction is SUSPENDED until \
                 the footprint grows past {:.2} GiB. Reclaiming space requires an offline \
                 rebuild (dump live entries into a fresh database and swap it in).",
                after as f64 / 1_073_741_824.0,
                (after.saturating_add(INEFFECTIVE_RETRY_MARGIN)) as f64 / 1_073_741_824.0,
            );
        } else {
            self.ineffective_above = None;
        }
        Ok(())
    }

    pub fn flush(&self) -> Result<()> {
        self.db.flush()?;
        Ok(())
    }
}
