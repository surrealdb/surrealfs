mod common;

use common::create_test_fs;
use surrealfs_sim::{FaultConfig, InvariantChecker, SimConfig, SimulationEngine};

#[tokio::test]
async fn test_simulation_with_seed_invariants() {
    let fs = create_test_fs().await;

    let config = SimConfig {
        seed: 1337,
        max_ticks: 30,
        num_agents: 4,
        fault_config: FaultConfig {
            drop_rate: 0.1,
            reorder_rate: 0.05,
            stall_rate: 0.05,
            max_stall_ticks: 3,
        },
    };

    let engine = SimulationEngine::new(config);
    engine
        .run(&fs)
        .await
        .expect("Deterministic simulation failed invariants");
}

#[tokio::test]
async fn test_branch_isolation_under_simulation() {
    let fs = create_test_fs().await;

    // Base document
    fs.write_text("/project/spec.md", "Specification v1.0", None)
        .await
        .unwrap();

    // Fork branch
    fs.fork_workspace("main", "feature-y", "sim-agent")
        .await
        .unwrap();

    // Branch isolation checker
    InvariantChecker::check_branch_isolation(&fs, "/project/spec.md", "Specification v1.0")
        .await
        .unwrap();
}

#[tokio::test]
async fn test_virtual_clock_determinism() {
    use surrealfs_sim::{EventQueue, VirtualClock};

    let clock = VirtualClock::new();
    let mut q = EventQueue::new();

    q.schedule(10, 1, "task-10");
    q.schedule(5, 2, "task-5");
    q.schedule(20, 1, "task-20");

    assert_eq!(q.pop_ready(clock.now()), None);

    clock.advance(5);
    assert_eq!(q.pop_ready(clock.now()), Some("task-5"));

    clock.advance(10);
    assert_eq!(q.pop_ready(clock.now()), Some("task-10"));

    clock.advance(10);
    assert_eq!(q.pop_ready(clock.now()), Some("task-20"));
    assert!(q.is_empty());
}
