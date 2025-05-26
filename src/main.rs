use clap::{Parser, Subcommand};
use std::io;
use std::sync::mpsc;

mod protocol;
mod source;
mod stats;
mod target;
mod telemetry;

/// UDP LZ4 Source/Target application with unicast and multicast support
#[derive(Parser)]
#[command(
    name = "iracing-teleport",
    version = env!("CARGO_PKG_VERSION"),
    author = env!("CARGO_PKG_AUTHORS"),
    about = env!("CARGO_PKG_DESCRIPTION"),
    after_help = "Visit https://github.com/sklose/iracing-teleport for more information."
)]
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

    let (shutdown_tx, shutdown_rx) = mpsc::channel();

    ctrlc::set_handler(move || {
        println!("Received Ctrl+C, shutting down...");
        let _ = shutdown_tx.send(());
    })
    .expect("Error setting Ctrl+C handler");

    match cli.mode {
        Mode::Source {
            bind,
            target,
            unicast,
        } => source::run(&bind, &target, unicast, shutdown_rx).inspect_err(|e| {
            eprintln!("Error in source: {}", e);
        }),

        Mode::Target {
            bind,
            group,
            unicast,
        } => target::run(&bind, unicast, group, shutdown_rx).inspect_err(|e| {
            eprintln!("Error in target: {}", e);
        }),
    }
}
