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

    pub fn call<T>(&self, make: impl FnOnce(Sender<SResult<T>>) -> StoreCmd) -> SResult<T> {
        let (tx, rx) = bounded(1);
        self.send(make(tx));
        rx.recv_timeout(std::time::Duration::from_secs(10)).map_err(|_| StorageError::Write("storage did not respond".into()))?
    }
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
                    None => repo.insert_run_intent(&i),
                };
                let _ = reply.send(r);
            }
            StoreCmd::Batch { run_id, batch } => {
                let result = match fault(&mut faults) {
                    Some(e) => Err(e),
                    None => repo.append_batch(&run_id, &batch),
                };
                let _ = replies.send(StoreReply::BatchAck { run_id, batch_seq: batch.batch_seq, result });
            }
            StoreCmd::Finalize(f) => {
                let result = match fault(&mut faults) {
                    Some(e) => Err(e),
                    None => repo.finalize_run(&f),
                };
                if let Err(e) = &result {
                    let _ = repo.note_persist_failed(&f.run_id, &e.to_string());
                }
                let _ = replies.send(StoreReply::Finalized { run_id: f.run_id.clone(), result });
            }
            StoreCmd::Revision(r, reply) => {
                let _ = reply.send(match fault(&mut faults) {
                    Some(e) => Err(e),
                    None => repo.append_revision(&r),
                });
            }
            StoreCmd::Calibration(c, reply) => {
                let _ = reply.send(repo.insert_calibration(&c));
            }
            StoreCmd::Session { shooter, new_id, now_utc_ms, tz, reply } => {
                let _ = reply.send(repo.active_session(&shooter, &new_id, now_utc_ms, tz));
            }
            StoreCmd::InjectFaults(n) => faults = n,
            StoreCmd::Shutdown => return,
        }
    }
}
