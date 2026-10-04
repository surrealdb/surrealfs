use rand::{Rng, SeedableRng};
use rand_chacha::ChaCha8Rng;
use std::sync::Mutex;

#[derive(Debug, Clone)]
pub struct FaultConfig {
    pub drop_rate: f64,
    pub reorder_rate: f64,
    pub stall_rate: f64,
    pub max_stall_ticks: u64,
}

impl Default for FaultConfig {
    fn default() -> Self {
        Self {
            drop_rate: 0.05,
            reorder_rate: 0.10,
            stall_rate: 0.08,
            max_stall_ticks: 5,
        }
    }
}

pub struct FaultInjector {
    rng: Mutex<ChaCha8Rng>,
    config: FaultConfig,
}

impl FaultInjector {
    pub fn new(seed: u64, config: FaultConfig) -> Self {
        Self {
            rng: Mutex::new(ChaCha8Rng::seed_from_u64(seed)),
            config,
        }
    }

    pub fn should_drop(&self) -> bool {
        let mut rng = self.rng.lock().unwrap();
        rng.gen_bool(self.config.drop_rate.clamp(0.0, 1.0))
    }

    pub fn should_reorder(&self) -> bool {
        let mut rng = self.rng.lock().unwrap();
        rng.gen_bool(self.config.reorder_rate.clamp(0.0, 1.0))
    }

    pub fn stall_ticks(&self) -> u64 {
        let mut rng = self.rng.lock().unwrap();
        if rng.gen_bool(self.config.stall_rate.clamp(0.0, 1.0)) {
            rng.gen_range(1..=self.config.max_stall_ticks.max(1))
        } else {
            0
        }
    }

    pub fn gen_range(&self, start: u64, end: u64) -> u64 {
        let mut rng = self.rng.lock().unwrap();
        if start >= end {
            start
        } else {
            rng.gen_range(start..end)
        }
    }

    pub fn choose_weighted<T: Clone>(&self, choices: &[(T, u32)]) -> Option<T> {
        let total: u32 = choices.iter().map(|(_, w)| *w).sum();
        if total == 0 {
            return None;
        }
        let mut rng = self.rng.lock().unwrap();
        let mut roll = rng.gen_range(0..total);
        for (item, weight) in choices {
            if roll < *weight {
                return Some(item.clone());
            }
            roll -= *weight;
        }
        choices.last().map(|(item, _)| item.clone())
    }
}
