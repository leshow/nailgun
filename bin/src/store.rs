use std::{
    collections::VecDeque,
    sync::{
        atomic::{self, AtomicU64},
        Arc,
    },
    time::{Duration, Instant},
};

use rand::prelude::SliceRandom;
use rustc_hash::FxHashMap;

#[derive(Debug)]
pub struct QueryInfo {
    pub sent: Instant,
    pub len: usize,
}

#[derive(Debug)]
pub struct Store {
    pub ids: VecDeque<u16>,
    pub in_flight: FxHashMap<u16, QueryInfo>,
}

impl Store {
    pub fn new() -> Self {
        Self {
            in_flight: FxHashMap::default(),
            ids: create_and_shuffle(),
        }
    }
    pub fn clear_timeouts(&mut self, timeout: Duration, atomic_store: &Arc<AtomicStore>) {
        let now = Instant::now();
        let mut ids = Vec::new();
        // remove all timed out ids from in_flight
        self.in_flight.retain(|id, info| {
            if now - info.sent >= timeout {
                ids.push(*id);
                false
            } else {
                true
            }
        });
        atomic_store
            .timed_out
            .fetch_add(ids.len() as u64, atomic::Ordering::Relaxed);

        // add back the ids so they can be used
        self.ids.extend(ids);
    }
}

#[derive(Debug, Default)]
pub struct AtomicStore {
    pub sent: AtomicU64,
    pub timed_out: AtomicU64,
}

impl AtomicStore {
    pub fn reset(&self) {
        self.sent.store(0, atomic::Ordering::Relaxed);
        self.timed_out.store(0, atomic::Ordering::Relaxed);
    }
}

// create a stack array of random u16's
fn create_and_shuffle() -> VecDeque<u16> {
    let mut data: Vec<u16> = (0..u16::max_value()).collect();
    data.shuffle(&mut rand::thread_rng());
    VecDeque::from(data)
}
