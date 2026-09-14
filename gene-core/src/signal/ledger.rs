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
    /// Set once the ledger has stopped accepting writes.
    ///
    /// THE QUOTA IS A CEILING, NOT A HINT. Before this existed, `append` grew
    /// the file forever whenever compaction could not reclaim: the suspension
    /// guard above stopped the pointless compaction, but nothing stopped the
    /// writes. The ledger went 46.4 -> 75.7 GiB in four days, filled the root
    /// filesystem, and crash-looped observer-gene 17,280 times over three days.
    /// Sealing is what makes the quota real.
    sealed: Option<SealReason>,
    /// Appends since the last size check while sealed, so a sealed ledger still
    /// notices if an operator frees space, without stat-ing on every tick.
    since_seal_check: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SealReason {
    /// Over quota and compaction cannot reclaim.
    QuotaExhausted,
    /// The underlying store refused a write (disk full, permissions, corruption).
    WriteFailed,
}

impl SealReason {
    fn as_str(self) -> &'static str {
        match self {
            SealReason::QuotaExhausted => "quota exhausted and compaction cannot reclaim",
            SealReason::WriteFailed => "the store refused a write",
        }
    }
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
            sealed: None,
            since_seal_check: 0,
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

    /// True once the ledger has stopped accepting writes.
    pub fn is_sealed(&self) -> bool {
        self.sealed.is_some()
    }

    /// Append a snapshot.
    ///
    /// NEVER returns Err for a storage problem. This ledger is an archive that
    /// nothing reads back — `append` and `flush` are its only callers in the
    /// whole workspace — so a failure to persist must not take down the agent
    /// that produces the data. Propagating ENOSPC from here is precisely how a
    /// full disk turned into 17,280 restarts of observer-gene: the process
    /// exited on `ledger.append(&snapshot)?` and systemd restarted it forever.
    /// Serialization bugs still surface as Err, because those are our own.
    pub fn append(&mut self, snapshot: &SignalSnapshot) -> Result<()> {
        if self.sealed.is_some() {
            self.recheck_seal();
            return Ok(());
        }

        let key = snapshot.tick.to_be_bytes();
        let value = bincode::serialize(snapshot)?;

        if let Err(e) = self.db.insert(key, value) {
            self.seal(SealReason::WriteFailed, Some(&e.to_string()));
            return Ok(());
        }
        self.since_size_check += 1;

        if self.since_size_check >= SIZE_CHECK_INTERVAL {
            self.since_size_check = 0;
            self.enforce_quota();
        }

        Ok(())
    }

    /// Bring the footprint back under quota, or seal if that is impossible.
    fn enforce_quota(&mut self) {
        let on_disk = self.size_on_disk();
        if on_disk <= self.quota_bytes {
            return;
        }

        let worth_trying = match self.ineffective_above {
            // Compaction already proved unable to reclaim at this size. Only
            // try again once the footprint has genuinely grown past it.
            Some(mark) => on_disk > mark.saturating_add(INEFFECTIVE_RETRY_MARGIN),
            None => true,
        };

        if worth_trying {
            if let Err(e) = self.compact(self.compact_batch, on_disk) {
                self.seal(SealReason::WriteFailed, Some(&e.to_string()));
                return;
            }
        }

        // The decisive check: if the footprint is STILL over quota after doing
        // everything we can, stop writing. Growing past the ceiling is not an
        // option — that is what filled the disk.
        if self.size_on_disk() > self.quota_bytes {
            self.seal(SealReason::QuotaExhausted, None);
        }
    }

    fn seal(&mut self, reason: SealReason, detail: Option<&str>) {
        if self.sealed.is_some() {
            return;
        }
        self.sealed = Some(reason);
        self.since_seal_check = 0;
        tracing::error!(
            "ledger SEALED — no further snapshots will be persisted. Reason: {}{}.              On disk {:.2} GiB against a {:.2} GiB quota. observer-gene keeps running:              nothing reads this ledger back, so losing new archive entries costs the              agent nothing, whereas filling the disk takes the whole host down.              To restore archiving, free space (the ledger directory can be removed              outright — the agent's state lives in checkpoint.bin) and restart.",
            reason.as_str(),
            detail.map(|d| format!(" ({})", d)).unwrap_or_default(),
            self.size_on_disk() as f64 / 1_073_741_824.0,
            self.quota_bytes as f64 / 1_073_741_824.0,
        );
    }

    /// While sealed, occasionally check whether space was freed underneath us.
    fn recheck_seal(&mut self) {
        self.since_seal_check += 1;
        if self.since_seal_check < SIZE_CHECK_INTERVAL {
            return;
        }
        self.since_seal_check = 0;

        // Only a quota seal can clear itself; a refused write means the store
        // is in a state an operator needs to look at.
        if self.sealed != Some(SealReason::QuotaExhausted) {
            return;
        }
        let on_disk = self.size_on_disk();
        if on_disk <= self.quota_bytes {
            self.sealed = None;
            self.ineffective_above = None;
            tracing::info!(
                "ledger unsealed: footprint back to {:.2} GiB, under the {:.2} GiB quota —                  resuming snapshot persistence",
                on_disk as f64 / 1_073_741_824.0,
                self.quota_bytes as f64 / 1_073_741_824.0,
            );
        }
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

    /// Flush pending writes.
    ///
    /// Like `append`, a storage failure here is reported and swallowed rather
    /// than propagated: main.rs calls `ledger.flush()?` on the checkpoint path
    /// and at shutdown, and a full disk must not turn either into a crash.
    pub fn flush(&mut self) -> Result<()> {
        if let Err(e) = self.db.flush() {
            self.seal(SealReason::WriteFailed, Some(&e.to_string()));
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::signal::types::{SignalId, SignalSnapshot};

    /// Unique scratch directory per test, removed on drop.
    struct Scratch(std::path::PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            let nanos = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            let p = std::env::temp_dir().join(format!("gene-ledger-{tag}-{nanos}"));
            Self(p)
        }
        fn path(&self) -> &std::path::Path {
            &self.0
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn snap(tick: u64) -> SignalSnapshot {
        SignalSnapshot {
            tick,
            timestamp_ms: tick as i64,
            // Enough payload that a few thousand entries exceed a small quota.
            values: (0..64u32).map(|i| (SignalId(i), i as f64 * 1.5)).collect(),
            imbalance: 1.0,
        }
    }

    /// THE REGRESSION. The quota used to be advisory: when compaction could not
    /// reclaim, appends carried on and the file grew without limit. That filled
    /// the root filesystem and crash-looped observer-gene for three days.
    #[test]
    fn append_stops_growing_once_the_quota_cannot_be_met() {
        let dir = Scratch::new("quota");
        // 1 MiB quota with a tiny compaction batch: compaction cannot keep up,
        // which is exactly the condition that previously grew without bound.
        let mut led = SignalLedger::open(dir.path(), 1024 * 1024, 1).unwrap();

        for tick in 0..60_000u64 {
            led.append(&snap(tick)).unwrap();
            if led.is_sealed() {
                break;
            }
        }

        assert!(led.is_sealed(), "ledger must seal rather than grow past quota");

        let sealed_at = led.size_on_disk();
        for tick in 60_000..90_000u64 {
            led.append(&snap(tick)).unwrap();
        }
        let after = led.size_on_disk();

        assert!(
            after <= sealed_at + 1024 * 1024,
            "sealed ledger grew from {sealed_at} to {after} bytes — the seal is not holding",
        );
    }

    /// A sealed ledger must not take the agent down with it.
    #[test]
    fn appending_to_a_sealed_ledger_is_a_successful_no_op() {
        let dir = Scratch::new("noop");
        let mut led = SignalLedger::open(dir.path(), 1024 * 1024, 1).unwrap();
        for tick in 0..60_000u64 {
            led.append(&snap(tick)).unwrap();
            if led.is_sealed() {
                break;
            }
        }
        assert!(led.is_sealed());

        // Every one of these must be Ok. main.rs does `ledger.append(&snapshot)?`,
        // so an Err here is a process exit and a systemd restart loop.
        for tick in 100_000..100_100u64 {
            assert!(led.append(&snap(tick)).is_ok(), "sealed append returned Err");
        }
        assert!(led.flush().is_ok(), "sealed flush returned Err");
    }

    #[test]
    fn a_healthy_ledger_stays_unsealed_and_persists() {
        let dir = Scratch::new("healthy");
        // Quota far above what these few entries need.
        let mut led = SignalLedger::open(dir.path(), 512 * 1024 * 1024, 64).unwrap();
        for tick in 0..2_000u64 {
            led.append(&snap(tick)).unwrap();
        }
        assert!(!led.is_sealed(), "a ledger inside its quota must not seal");
        led.flush().unwrap();
        assert_eq!(led.tail(1).unwrap().len(), 1, "entries should be readable back");
    }

    #[test]
    fn reopening_a_ledger_does_not_scan_it() {
        // Guards the 38-minute startup: open() must not walk the database.
        let dir = Scratch::new("reopen");
        {
            let mut led = SignalLedger::open(dir.path(), 512 * 1024 * 1024, 64).unwrap();
            for tick in 0..5_000u64 {
                led.append(&snap(tick)).unwrap();
            }
            led.flush().unwrap();
        }
        let start = std::time::Instant::now();
        let led = SignalLedger::open(dir.path(), 512 * 1024 * 1024, 64).unwrap();
        assert!(!led.is_sealed());
        assert!(
            start.elapsed() < std::time::Duration::from_secs(5),
            "open() took {:?} — it is scanning the database again",
            start.elapsed(),
        );
    }
}
