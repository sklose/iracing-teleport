use std::time::{Duration, Instant};

// Statistics print interval
const STATS_INTERVAL: Duration = Duration::from_secs(30);

pub struct StatisticsPrinter {
    name: &'static str,
    start_time: Instant,
    updates: u32,
}

impl StatisticsPrinter {
    pub fn new(name: &'static str) -> Self {
        Self {
            name,
            start_time: Instant::now(),
            updates: 0,
        }
    }

    pub fn add_update(&mut self) {
        self.updates += 1;
    }

    pub fn print_and_reset(&mut self) {
        let elapsed = self.start_time.elapsed().as_secs_f64();
        let rate = self.updates as f64 / elapsed;
        println!("[{}] {:.2} msgs/s", self.name, rate);
        
        self.updates = 0;
        self.start_time = Instant::now();
    }

    pub fn should_print(&self) -> bool {
        self.start_time.elapsed() >= STATS_INTERVAL
    }
} 