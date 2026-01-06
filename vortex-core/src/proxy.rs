use crate::reactor::ShardReactor;
use log::info;
use std::thread;
use crossbeam_utils::sync::WaitGroup;

/// ShardProxy: Orchestrates multiple ShardReactors across cores.
/// 
/// # Responsibilities
/// 1. Spawning one OS thread per physical core.
/// 2. Pinning threads to their respective cores (Rule #7).
/// 3. Initializing the ShardReactor state.
/// 4. Managing the lifecycle (Spawn -> Run -> Shutdown).
pub struct ShardProxy {
    num_shards: usize,
}

impl ShardProxy {
    /// Initializes a new Proxy orchestrator.
    pub fn new(num_shards: usize) -> Self {
        Self { num_shards }
    }

    /// Spawns and pins all Shard Reactor threads.
    /// 
    /// # Arguments
    /// * `start_port` - The VBP Ingress port. All shards bind to this same port 
    ///                  using `SO_REUSEPORT` for hardware load balancing.
    pub fn spawn_shards(&self, start_port: u16) {
        let wg = WaitGroup::new();

        for i in 0..self.num_shards {
            let shard_id = i;
            let port = start_port;
            let wg = wg.clone();

            // Use Builder to name threads for easier htop/perf debugging.
            thread::Builder::new()
                .name(format!("shard_{}", shard_id))
                .spawn(move || {
                    // 1. Thread Pinning (CRITICAL: MUST BE FIRST)
                    vortex_io::platform::affinity::pin_thread_to_core(shard_id);
                    
                    // 2. Initialize Shard Reactor
                    let mut reactor = ShardReactor::new(shard_id, 256);
                    
                    // 3. Bind to Ingress Port
                    // Rule I Compliance: In startup phase, expect() is allowed.
                    if let Err(e) = reactor.listen(port) {
                        panic!("CRITICAL: Shard {} failed to bind port {}: {}", shard_id, port, e);
                    }
                    
                    info!("Shard {} Online. Pinned to Core {}. Listening on port {}.", shard_id, shard_id, port);
                    
                    // Signal readiness
                    drop(wg);

                    // 4. Enter Reactor Loop (Infinite)
                    // Rule IV: No logs in hot loop.
                    loop {
                        reactor.run_tick();
                    }
                })
                .expect("Failed to spawn OS thread");
        }

        // Wait for all shards to initialize and bind
        wg.wait();
        info!("Cluster Orchestrator: All {} shards spawned and running.", self.num_shards);
    }

    /// Signal a graceful shutdown to all shards.
    pub fn shutdown(&self) {
        info!("Proxy -> signal sent to all shards (Stub: Simulated Graceful Shutdown).");
    }
}
