use crate::checker::InvariantChecker;
use crate::fault::{FaultConfig, FaultInjector};
use crate::virtual_time::VirtualClock;
use anyhow::Result;
use surrealfs_core::SurrealFs;
use tracing::{info, warn};

#[derive(Debug, Clone)]
pub struct SimConfig {
    pub seed: u64,
    pub max_ticks: u64,
    pub num_agents: usize,
    pub fault_config: FaultConfig,
}

impl Default for SimConfig {
    fn default() -> Self {
        Self {
            seed: 42,
            max_ticks: 50,
            num_agents: 3,
            fault_config: FaultConfig::default(),
        }
    }
}

pub struct SimulationEngine {
    pub config: SimConfig,
    pub clock: VirtualClock,
    pub injector: FaultInjector,
}

impl SimulationEngine {
    pub fn new(config: SimConfig) -> Self {
        let injector = FaultInjector::new(config.seed, config.fault_config.clone());
        let clock = VirtualClock::new();
        Self {
            config,
            clock,
            injector,
        }
    }

    /// Run the deterministic simulation against an active SurrealFs instance.
    pub async fn run(&self, fs: &SurrealFs) -> Result<()> {
        info!(
            "Starting deterministic simulation with seed {}",
            self.config.seed
        );

        let paths = [
            "/sim/doc_a.txt",
            "/sim/sub/doc_b.txt",
            "/sim/sub/deep/doc_c.txt",
        ];

        // Setup base directory
        fs.mkdir("/sim/sub/deep", true).await?;

        for tick in 0..self.config.max_ticks {
            self.clock.advance(1);

            let agent_idx = (self.injector.gen_range(0, self.config.num_agents as u64)) as usize;
            let agent_id = format!("agent-{}", agent_idx);
            let path = paths[(self.injector.gen_range(0, paths.len() as u64)) as usize];

            // Simulated operation weights:
            // 0: Write/Append
            // 1: Lock/Unlock
            // 2: Read/Stat
            let op_choice = self.injector.gen_range(0, 3);

            match op_choice {
                0 => {
                    if !self.injector.should_drop() {
                        let content = format!("Written at tick {} by {}\n", tick, agent_id);
                        if fs.exists(path).await.unwrap_or(false) {
                            let _ = fs.append_text(path, &content).await;
                        } else {
                            let _ = fs.write_text(path, &content, None).await;
                        }
                    } else {
                        warn!("Tick {}: Packet dropped for write op on {}", tick, path);
                    }
                }
                1 => {
                    if !self.injector.should_drop() {
                        let stall = self.injector.stall_ticks();
                        if stall > 0 {
                            self.clock.advance(stall);
                        }
                        if let Ok(lock) = fs.acquire_lock(path, 30, "sim_lease", &agent_id).await {
                            let _ = fs.release_lock(path, &lock.holder).await;
                        }
                    }
                }
                2 => {
                    let _ = fs.stat(path).await;
                }
                _ => {}
            }

            // Continuous Invariant Checking
            if tick % 5 == 0 {
                InvariantChecker::check_tree_hierarchy(fs).await?;
                InvariantChecker::check_lock_exclusivity(fs).await?;
            }
        }

        // Final thorough check
        InvariantChecker::check_tree_hierarchy(fs).await?;
        InvariantChecker::check_lock_exclusivity(fs).await?;

        info!(
            "Simulation successfully passed all invariants for seed {}",
            self.config.seed
        );
        Ok(())
    }
}
