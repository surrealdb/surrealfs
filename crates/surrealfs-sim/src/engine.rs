use crate::checker::InvariantChecker;
use crate::fault::{FaultConfig, FaultInjector};
use crate::virtual_time::{EventQueue, VirtualClock};
use anyhow::Result;
use surrealfs_core::SurrealFs;
use tracing::{info, warn};

#[derive(Debug, Clone, Eq, PartialEq)]
enum SimOp {
    Write { path: String, content: String },
    LockUnlock { path: String, agent_id: String },
    Stat { path: String },
    CopyVerify { src: String, dst: String },
}

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

    async fn execute_op(&self, fs: &SurrealFs, op: SimOp) {
        match op {
            SimOp::Write { path, content } => {
                if fs.exists(&path).await.unwrap_or(false) {
                    let _ = fs.append_text(&path, &content).await;
                } else {
                    let _ = fs.write_text(&path, &content, None).await;
                    let _ = fs.chmod(&path, 0o666).await;
                }
            }
            SimOp::LockUnlock { path, agent_id } => {
                if let Ok(lock) = fs.acquire_lock(&path, 30, "sim_lease", &agent_id).await {
                    let _ = fs.release_lock(&path, &lock.holder).await;
                }
            }
            SimOp::Stat { path } => {
                let _ = fs.stat(&path).await;
            }
            SimOp::CopyVerify { src, dst } => {
                if fs.exists(&src).await.unwrap_or(false) {
                    let _ = fs.cp(&src, &dst, false).await;
                }
            }
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

        // Setup base directory and anchor file for branch isolation verification
        fs.mkdir("/sim/sub/deep", true).await?;
        let base_anchor_path = "/sim/anchor.txt";
        let base_anchor_content = "immutable-main-anchor\n";
        fs.write_text(base_anchor_path, base_anchor_content, None)
            .await?;

        let mut queue: EventQueue<SimOp> = EventQueue::new();

        for tick in 0..self.config.max_ticks {
            let now = self.clock.advance(1);

            let agent_idx = (self.injector.gen_range(0, self.config.num_agents as u64)) as usize;
            let agent_id = format!("agent-{}", agent_idx);
            let path = paths[(self.injector.gen_range(0, paths.len() as u64)) as usize];

            // Simulated operation weights:
            // 0: Write/Append
            // 1: Lock/Unlock
            // 2: Read/Stat
            // 3: Copy within /sim/sub
            let op_choice = self.injector.gen_range(0, 4);

            if !self.injector.should_drop() {
                let op = match op_choice {
                    0 => SimOp::Write {
                        path: path.to_string(),
                        content: format!("Written at tick {} by {}\n", tick, agent_id),
                    },
                    1 => SimOp::LockUnlock {
                        path: path.to_string(),
                        agent_id: agent_id.clone(),
                    },
                    2 => SimOp::Stat {
                        path: path.to_string(),
                    },
                    _ => SimOp::CopyVerify {
                        src: path.to_string(),
                        dst: format!("/sim/sub/copy_{}.txt", agent_idx),
                    },
                };

                let stall = self.injector.stall_ticks();
                let reorder_delay = if self.injector.should_reorder() { 1 } else { 0 };
                let target_tick = now + stall + reorder_delay;
                let priority = if reorder_delay > 0 { 1 } else { 10 };
                queue.schedule(target_tick, priority, op);
            } else {
                warn!("Tick {}: Packet dropped for op on {}", tick, path);
            }

            while let Some(ready_op) = queue.pop_ready(self.clock.now()) {
                self.execute_op(fs, ready_op).await;
            }

            // Continuous Invariant Checking
            if tick % 5 == 0 {
                InvariantChecker::check_tree_hierarchy(fs).await?;
                InvariantChecker::check_lock_exclusivity(fs).await?;
                InvariantChecker::check_branch_isolation(
                    fs,
                    base_anchor_path,
                    base_anchor_content,
                )
                .await?;
            }
        }

        // Drain any remaining delayed events
        let final_tick = self.clock.advance(self.config.fault_config.max_stall_ticks + 2);
        while let Some(ready_op) = queue.pop_ready(final_tick) {
            self.execute_op(fs, ready_op).await;
        }

        // Final thorough check
        InvariantChecker::check_tree_hierarchy(fs).await?;
        InvariantChecker::check_lock_exclusivity(fs).await?;
        InvariantChecker::check_branch_isolation(fs, base_anchor_path, base_anchor_content).await?;

        info!(
            "Simulation successfully passed all invariants for seed {}",
            self.config.seed
        );
        Ok(())
    }
}
