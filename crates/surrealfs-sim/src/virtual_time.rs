use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::sync::Arc;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct ScheduledEvent<T> {
    pub tick: u64,
    pub priority: u32,
    pub payload: T,
}

impl<T: Eq + PartialEq> Ord for ScheduledEvent<T> {
    fn cmp(&self, other: &Self) -> Ordering {
        // Min-heap: earliest tick first
        other
            .tick
            .cmp(&self.tick)
            .then_with(|| other.priority.cmp(&self.priority))
    }
}

impl<T: Eq + PartialEq> PartialOrd for ScheduledEvent<T> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Clone)]
pub struct VirtualClock {
    current_tick: Arc<AtomicU64>,
}

impl VirtualClock {
    pub fn new() -> Self {
        Self {
            current_tick: Arc::new(AtomicU64::new(0)),
        }
    }

    pub fn now(&self) -> u64 {
        self.current_tick.load(AtomicOrdering::SeqCst)
    }

    pub fn advance(&self, ticks: u64) -> u64 {
        self.current_tick.fetch_add(ticks, AtomicOrdering::SeqCst) + ticks
    }

    pub fn reset(&self) {
        self.current_tick.store(0, AtomicOrdering::SeqCst);
    }
}

impl Default for VirtualClock {
    fn default() -> Self {
        Self::new()
    }
}

pub struct EventQueue<T> {
    heap: BinaryHeap<ScheduledEvent<T>>,
}

impl<T: Eq + PartialEq> EventQueue<T> {
    pub fn new() -> Self {
        Self {
            heap: BinaryHeap::new(),
        }
    }

    pub fn schedule(&mut self, tick: u64, priority: u32, payload: T) {
        self.heap.push(ScheduledEvent {
            tick,
            priority,
            payload,
        });
    }

    pub fn pop_ready(&mut self, current_tick: u64) -> Option<T> {
        if let Some(event) = self.heap.peek() {
            if event.tick <= current_tick {
                return self.heap.pop().map(|e| e.payload);
            }
        }
        None
    }

    pub fn is_empty(&self) -> bool {
        self.heap.is_empty()
    }

    pub fn len(&self) -> usize {
        self.heap.len()
    }
}

impl<T: Eq + PartialEq> Default for EventQueue<T> {
    fn default() -> Self {
        Self::new()
    }
}
