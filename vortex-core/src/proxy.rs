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
    max_elements_per_shard: usize,
}

impl ShardProxy {
    /// Initializes a new Proxy orchestrator.
    pub fn new(num_shards: usize, max_elements_per_shard: usize) -> Self {
        Self { num_shards, max_elements_per_shard }
    }

    /// Spawns and pins all Shard Reactor threads.
    /// 
    /// # Arguments
    /// * `start_port` - The VBP Ingress port. All shards bind to this same port 
    ///                  using `SO_REUSEPORT` for hardware load balancing.
    pub fn spawn_shards(&self, start_port: u16) {
        let wg = WaitGroup::new();

        // If num_shards > 1, spawn n-1 shards in threads.
        // The last shard (or the only shard) will run on the calling thread.
        let background_shards = if self.num_shards > 1 { self.num_shards - 1 } else { 0 };

        let mut actually_spawned = 0;
        for i in 0..background_shards {
            let shard_id = i;
            let port = start_port;
            let wg = wg.clone();
            let max_el = self.max_elements_per_shard;

            let result = thread::Builder::new()
                .name(format!("shard_{}", shard_id))
                .stack_size(512 * 1024) // 512KB stack (Termux Friendly)
                .spawn(move || {
                    vortex_io::platform::affinity::pin_thread_to_core(shard_id);
                    let mut reactor = ShardReactor::new(shard_id, 256, max_el);
                    if let Err(e) = reactor.listen(port) {
                        panic!("CRITICAL: Shard {} failed to bind port {}: {}", shard_id, port, e);
                    }
                    info!("Shard {} Online (Threaded). Pinned to Core {}.", shard_id, shard_id);
                    drop(wg);
                    loop { reactor.run_tick(); }
                });
            
            match result {
                Ok(_) => { actually_spawned += 1; },
                Err(e) => {
                    log::warn!("OS Refused More Threads: {}. Stopping background shard spawning.", e);
                    break;
                }
            }
        }

        // Run the final shard on the MAIN thread to avoid EAGAIN on constrained systems
        let main_shard_id = self.num_shards - 1;
        let port = start_port;
        let max_el = self.max_elements_per_shard;
        
        info!("Shard {} Online (Main Thread Fallback). Listening on port {}.", main_shard_id, port);
        
        // This shard must handle its own pinning and setup
        vortex_io::platform::affinity::pin_thread_to_core(main_shard_id);
        let mut reactor = ShardReactor::new(main_shard_id, 256, max_el);
        reactor.listen(port).expect("Main shard bind failed");

        // Signal cluster readiness if others are waiting (Wait for those that actually spawned)
        // WaitGroup drops for each successful spawn. We need to drop the remaining ones.
        for _ in 0..(background_shards - actually_spawned) {
            drop(wg.clone());
        }
        wg.wait();
        info!("Cluster Orchestrator: All {} active shards online (Requested: {}).", actually_spawned + 1, self.num_shards);

        // Enter Main Loop for Shard N-1
        loop {
            reactor.run_tick();
        }
    }

    /// Signal a graceful shutdown to all shards.
    pub fn shutdown(&self) {
        info!("Proxy -> signal sent to all shards (Stub: Simulated Graceful Shutdown).");
    }
}
