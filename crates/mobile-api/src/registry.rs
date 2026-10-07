//! Data-plane handle registry. Kotlin holds an opaque `u64`; a stale or unknown handle
//! is rejected instead of dereferencing freed memory.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, OnceLock, RwLock};
use std::thread::Thread;

use crate::queue::PcmQueue;

pub struct Sink {
    pub queue: Arc<PcmQueue>,
    pub worker: Option<Thread>,
}

static NEXT: AtomicU64 = AtomicU64::new(1);

fn map() -> &'static RwLock<HashMap<u64, Arc<Sink>>> {
    static M: OnceLock<RwLock<HashMap<u64, Arc<Sink>>>> = OnceLock::new();
    M.get_or_init(|| RwLock::new(HashMap::new()))
}

pub fn register(queue: Arc<PcmQueue>, worker: Option<Thread>) -> u64 {
    let h = NEXT.fetch_add(1, Ordering::Relaxed);
    map().write().unwrap_or_else(|p| p.into_inner()).insert(h, Arc::new(Sink { queue, worker }));
    h
}

pub fn unregister(handle: u64) {
    map().write().unwrap_or_else(|p| p.into_inner()).remove(&handle);
}

pub fn get(handle: u64) -> Option<Arc<Sink>> {
    map().read().unwrap_or_else(|p| p.into_inner()).get(&handle).cloned()
}
