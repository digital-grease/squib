//! Repository actor: one writer thread owns the write connection. Commands are
//! processed in order; batch and final-commit results return asynchronously so the
//! control path never waits on disk while a run is active.

use std::path::PathBuf;
use std::thread::JoinHandle;

use crossbeam_channel::{Receiver, Sender, bounded, unbounded};
use squib_domain::{ReviewState, RunRevision};
use squib_storage::{
    CalibrationRecord, FinalRecord, ObservationBatch, RecoveredRun, Repository, Result as SResult, RunIntent, StorageError,
};

pub enum StoreCmd {
    Intent(Box<RunIntent>, Sender<SResult<()>>),
    Batch {
        run_id: String,
        batch: Box<ObservationBatch>,
    },
    Finalize(Box<FinalRecord>),
    Revision(Box<RunRevision>, Sender<SResult<ReviewState>>),
    Calibration(Box<CalibrationRecord>, Sender<SResult<()>>),
    Session {
        shooter: String,
        new_id: String,
        now_utc_ms: i64,
        tz: i32,
        reply: Sender<SResult<String>>,
    },
    /// Run an arbitrary repository operation on the writer thread (conditions, places,
    /// settings). Keeps the single-writer rule without a message per operation.
    Exec(Box<dyn FnOnce(&mut Repository) + Send>),
    /// Fail the next `n` write commands with a simulated write error (fault injection).
    InjectFaults(u32),
    Shutdown,
}

#[derive(Debug)]
pub enum StoreReply {
    BatchAck { run_id: String, batch_seq: u64, result: SResult<u64> },
    Finalized { run_id: String, result: SResult<()> },
}

pub struct StoreActor {
    tx: Sender<StoreCmd>,
    pub replies: Receiver<StoreReply>,
    thread: Option<JoinHandle<()>>,
}

impl StoreActor {
    /// Opens the database (running migrations and recovery) before returning.
    pub fn start(path: PathBuf, now_utc_ms: i64) -> SResult<(Self, Vec<RecoveredRun>)> {
        let (tx, rx) = unbounded::<StoreCmd>();
        let (rtx, rrx) = unbounded::<StoreReply>();
        let (ready_tx, ready_rx) = bounded::<SResult<Vec<RecoveredRun>>>(1);
        let thread = std::thread::Builder::new()
            .name("squib-store".into())
            .spawn(move || {
                let mut repo = match Repository::open(&path) {
                    Ok(r) => r,
                    Err(e) => {
                        let _ = ready_tx.send(Err(e));
                        return;
                    }
                };
                let rec = repo.ensure_default_shooter(now_utc_ms).and_then(|_| repo.recover_unfinished(now_utc_ms));
                let _ = ready_tx.send(rec);
                actor_loop(repo, rx, rtx);
            })
            .map_err(|e| StorageError::Write(e.to_string()))?;
        let recovered = ready_rx.recv().map_err(|e| StorageError::Write(e.to_string()))??;
        Ok((Self { tx, replies: rrx, thread: Some(thread) }, recovered))
    }

    pub fn send(&self, c: StoreCmd) {
        let _ = self.tx.send(c);
    }

    /// Run `f` on the writer thread and wait for its result.
    /// A sender for work queued from another thread (the writer order is kept).
    pub fn sender(&self) -> Sender<StoreCmd> {
        self.tx.clone()
    }

    pub fn exec<T: Send + 'static>(&self, f: impl FnOnce(&mut Repository) -> SResult<T> + Send + 'static) -> SResult<T> {
        let (tx, rx) = bounded(1);
        self.send(StoreCmd::Exec(Box::new(move |repo| {
            let _ = tx.send(f(repo));
        })));
        rx.recv_timeout(std::time::Duration::from_secs(10)).map_err(no_reply)?
    }

    pub fn call<T>(&self, make: impl FnOnce(Sender<SResult<T>>) -> StoreCmd) -> SResult<T> {
        let (tx, rx) = bounded(1);
        self.send(make(tx));
        rx.recv_timeout(std::time::Duration::from_secs(10)).map_err(no_reply)?
    }
}

fn no_reply(e: crossbeam_channel::RecvTimeoutError) -> StorageError {
    StorageError::Write(match e {
        crossbeam_channel::RecvTimeoutError::Timeout => "storage did not respond".into(),
        crossbeam_channel::RecvTimeoutError::Disconnected => "internal storage error; nothing was saved".into(),
    })
}

impl Drop for StoreActor {
    fn drop(&mut self) {
        let _ = self.tx.send(StoreCmd::Shutdown);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn actor_loop(mut repo: Repository, rx: Receiver<StoreCmd>, replies: Sender<StoreReply>) {
    let mut faults = 0u32;
    let fault = |faults: &mut u32| -> Option<StorageError> {
        if *faults > 0 {
            *faults -= 1;
            Some(StorageError::Write("injected write failure".into()))
        } else {
            None
        }
    };
    while let Ok(cmd) = rx.recv() {
        match cmd {
            StoreCmd::Intent(i, reply) => {
                let r = match fault(&mut faults) {
                    Some(e) => Err(e),
                    None => guarded(&mut repo, |r| r.insert_run_intent(&i)),
                };
                let _ = reply.send(r);
            }
            StoreCmd::Batch { run_id, batch } => {
                let result = match fault(&mut faults) {
                    Some(e) => Err(e),
                    None => guarded(&mut repo, |r| r.append_batch(&run_id, &batch)),
                };
                let _ = replies.send(StoreReply::BatchAck { run_id, batch_seq: batch.batch_seq, result });
            }
            StoreCmd::Finalize(f) => {
                let result = match fault(&mut faults) {
                    Some(e) => Err(e),
                    None => guarded(&mut repo, |r| r.finalize_run(&f)),
                };
                if let Err(e) = &result {
                    let _ = guarded(&mut repo, |r| r.note_persist_failed(&f.run_id, &e.to_string()));
                }
                let _ = replies.send(StoreReply::Finalized { run_id: f.run_id.clone(), result });
            }
            StoreCmd::Revision(r, reply) => {
                let _ = reply.send(match fault(&mut faults) {
                    Some(e) => Err(e),
                    None => guarded(&mut repo, |repo| repo.append_revision(&r)),
                });
            }
            StoreCmd::Calibration(c, reply) => {
                let _ = reply.send(guarded(&mut repo, |r| r.insert_calibration(&c)));
            }
            StoreCmd::Session { shooter, new_id, now_utc_ms, tz, reply } => {
                let _ = reply.send(guarded(&mut repo, |r| r.active_session(&shooter, &new_id, now_utc_ms, tz)));
            }
            // A panicking operation drops its reply sender, so the caller gets an error
            // at once; the writer thread and its connection carry on.
            StoreCmd::Exec(f) => {
                let _ = guarded(&mut repo, |r| {
                    f(r);
                    Ok(())
                });
            }
            StoreCmd::InjectFaults(n) => faults = n,
            StoreCmd::Shutdown => return,
        }
    }
}

/// Run one repository operation, containing a panic: an open transaction is rolled back
/// and the operation reports a write error instead of killing the writer thread, which
/// would otherwise fail every later save for the rest of the process.
fn guarded<T>(repo: &mut Repository, f: impl FnOnce(&mut Repository) -> SResult<T>) -> SResult<T> {
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f(repo))) {
        Ok(r) => r,
        Err(_) => {
            let c = repo.connection();
            if !c.is_autocommit() {
                let _ = c.execute_batch("ROLLBACK");
            }
            Err(StorageError::Write("internal storage error; nothing was saved".into()))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    fn actor(name: &str) -> StoreActor {
        let d = std::env::temp_dir().join(format!("squib-store-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        StoreActor::start(d.join("j.db"), 1).unwrap().0
    }

    #[test]
    fn a_panicking_operation_fails_at_once_and_the_writer_carries_on() {
        let a = actor("panic");
        let t = Instant::now();
        let e = a.exec(|_| -> SResult<()> { panic!("boom") }).unwrap_err();
        assert!(t.elapsed() < Duration::from_secs(2), "reported at once, not after the timeout");
        assert!(e.to_string().contains("internal storage error"), "{e}");
        a.exec(|r| r.set_setting("k", "v")).unwrap();
        assert_eq!(a.exec(|r| r.get_setting("k")).unwrap().as_deref(), Some("v"), "writer thread still alive");
    }

    #[test]
    fn a_panic_inside_a_transaction_is_rolled_back() {
        let a = actor("rollback");
        let e = a.exec(|r| -> SResult<()> {
            r.connection().execute_batch("BEGIN; INSERT INTO app_setting(key, value) VALUES ('half', 'written');").unwrap();
            panic!("boom mid-transaction")
        });
        assert!(e.is_err());
        assert_eq!(a.exec(|r| r.get_setting("half")).unwrap(), None, "partial write rolled back");
        a.exec(|r| r.set_setting("after", "ok")).unwrap();
        assert_eq!(
            a.exec(|r| r.get_setting("after")).unwrap().as_deref(),
            Some("ok"),
            "connection usable, not stuck in a transaction"
        );
    }
}
