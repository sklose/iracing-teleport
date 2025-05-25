use clap::{Parser, Subcommand};
use std::io;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

mod source;
mod target;
mod telemetry;

/// UDP LZ4 Source/Target application with unicast and multicast support
#[derive(Parser)]
#[command(name = "UDP LZ4 Source/Target", version, about)]
struct Cli {
    #[command(subcommand)]
    mode: Mode,
}

#[derive(Subcommand)]
enum Mode {
    /// Run as the source (sends compressed data at 60Hz)
    Source {
        /// Local bind address (e.g., 127.0.0.1:5000)
        #[arg(long, default_value = "0.0.0.0:0")]
        bind: String,

        /// Target address to send data to (e.g., 127.0.0.1:5000)
        #[arg(long, default_value = "239.255.0.1:5000")]
        target: String,

        /// Use unicast mode instead of multicast
        #[arg(long)]
        unicast: bool,
    },

    /// Run as the target (receives compressed data)
    Target {
        /// Address to bind to for receiving (e.g., 127.0.0.1:5000)
        #[arg(long, default_value = "0.0.0.0:5000")]
        bind: String,

        /// Multicast group to join
        #[arg(long, default_value = "239.255.0.1")]
        group: String,

        /// Use unicast mode instead of multicast
        #[arg(long)]
        unicast: bool,
    },
}

fn main() -> io::Result<()> {
    let cli = Cli::parse();

    let running: Arc<AtomicBool> = Arc::new(AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        println!("Received Ctrl+C, shutting down...");
        r.store(false, Ordering::SeqCst);
    })
    .expect("Error setting Ctrl+C handler");

    match cli.mode {
        Mode::Source {
            bind,
            target,
            unicast,
        } => source::run(&bind, &target, unicast, running).map_err(|e| {
            eprintln!("Error in source: {}", e);
            io::Error::other("Source error")
        }),

        Mode::Target {
            bind,
            group,
            unicast,
        } => target::run(&bind, unicast, group, running),
    }
}
