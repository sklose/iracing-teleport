use std::time::{Duration, Instant};

// Statistics print interval
const STATS_INTERVAL: Duration = Duration::from_secs(5);

pub struct StatisticsPrinter {
    name: &'static str,
    start_time: Instant,
    updates: u32,
    total_bytes: u64,
    total_fragments: u64,
    total_latency_us: u64,
}

impl StatisticsPrinter {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            start_time: Instant::now(),
            updates: 0,
            total_bytes: 0,
            total_fragments: 0,
            total_latency_us: 0,
        }
    }

    pub fn add_update(&mut self) {
        self.updates += 1;
    }

    pub fn add_bytes(&mut self, count: usize) {
        self.total_bytes += count as u64;
    }

    pub fn add_fragments(&mut self, count: u16) {
        self.total_fragments += count as u64;
    }

    pub fn add_latency(&mut self, latency_us: u64) {
        self.total_latency_us += latency_us;
    }

    pub fn print_and_reset(&mut self) {
        let elapsed = self.start_time.elapsed().as_secs_f64();
        let rate = self.updates as f64 / elapsed;
        let mbps = (self.total_bytes as f64 * 8.0) / (elapsed * 1_000_000.0);
        let avg_fragments = if self.updates > 0 {
            self.total_fragments as f64 / self.updates as f64
        } else {
            0.0
        };
        let avg_latency = if self.updates > 0 {
            (self.total_latency_us as f64) / (self.updates as f64)
        } else {
            0.0
        };

        println!(
            "[{}] {:.2} msgs/s | Bandwidth: {:.2} Mbps | Avg fragments: {:.1} | Avg latency: {:.1} µs",
            self.name, rate, mbps, avg_fragments, avg_latency
        );

        self.updates = 0;
        self.total_bytes = 0;
        self.total_fragments = 0;
        self.total_latency_us = 0;
        self.start_time = Instant::now();
    }

    pub fn should_print(&self) -> bool {
        self.start_time.elapsed() >= STATS_INTERVAL
    }
}
