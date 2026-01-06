use crate::reactor::ShardReactor;
use log::info;
use std::thread;
use crossbeam_utils::sync::WaitGroup;

/// ShardProxy: Orchestrates multiple ShardReactors across cores.
pub struct ShardProxy {
    num_shards: usize,
}

impl ShardProxy {
    pub fn new(num_shards: usize) -> Self {
        Self { num_shards }
    }

    pub fn spawn_shards(&self, start_port: u16) {
        let wg = WaitGroup::new();

        for i in 0..self.num_shards {
            let shard_id = i;
            let port = start_port; // All shards share the same port!
            let addr = format!("0.0.0.0:{}", port);
            let wg = wg.clone();

            thread::spawn(move || {
                info!("Proxy -> Spawning Shard Reactor {} on port {}...", shard_id, port);
                
                // Rule 2: Pin to Core (shard_id corresponds to physical core)
                vortex_io::platform::affinity::pin_thread_to_core(shard_id);
                
                let mut reactor = ShardReactor::new(shard_id, 256);
                reactor.listen(&addr).expect("Failed to bind shard port");
                
                info!("Shard {} Active and Listening.", shard_id);
                drop(wg);

                loop {
                    reactor.run_tick();
                }
            });
        }

        wg.wait();
        info!("All {} shards spawned and pinned.", self.num_shards);
    }
}
