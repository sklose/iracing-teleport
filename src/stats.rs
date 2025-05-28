use std::time::{Duration, Instant};

// Statistics print interval
const STATS_INTERVAL: Duration = Duration::from_secs(5);

pub struct StatisticsPrinter {
    name: &'static str,
    start_time: Instant,
    updates: u32,
    bytes: u64,
    protocol_bytes: u64,
    total_latency_us: u64,
}

impl StatisticsPrinter {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            start_time: Instant::now(),
            updates: 0,
            bytes: 0,
            protocol_bytes: 0,
            total_latency_us: 0,
        }
    }

    pub fn add_update(&mut self) {
        self.updates += 1;
    }

    pub fn add_bytes(&mut self, count: usize) {
        self.bytes += count as u64;
    }

    pub fn add_protocol_bytes(&mut self, count: usize) {
        self.protocol_bytes += count as u64;
    }

    pub fn add_latency(&mut self, latency_us: u64) {
        self.total_latency_us += latency_us;
    }

    pub fn print_and_reset(&mut self) {
        let elapsed = self.start_time.elapsed().as_secs_f64();
        let rate = self.updates as f64 / elapsed;
        let mbps = (self.bytes as f64 * 8.0) / (elapsed * 1_000_000.0);
        let protocol_mbps = (self.protocol_bytes as f64 * 8.0) / (elapsed * 1_000_000.0);
        let overhead = if self.bytes > 0 {
            ((self.protocol_bytes as f64 / self.bytes as f64) - 1.0) * 100.0
        } else {
            0.0
        };
        let avg_latency = if self.updates > 0 {
            (self.total_latency_us as f64) / (self.updates as f64)
        } else {
            0.0
        };

        println!(
            "[{}] {:.2} msgs/s | Data: {:.2} Mbps | Wire: {:.2} Mbps | Protocol overhead: {:.1}% | Avg latency: {:.1} µs",
            self.name, rate, mbps, protocol_mbps, overhead, avg_latency
        );

        self.updates = 0;
        self.bytes = 0;
        self.protocol_bytes = 0;
        self.total_latency_us = 0;
        self.start_time = Instant::now();
    }

    pub fn should_print(&self) -> bool {
        self.start_time.elapsed() >= STATS_INTERVAL
    }
}
