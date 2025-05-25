use clap::{Parser, Subcommand};
use lz4::block::{compress_to_buffer, decompress_to_buffer};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::{
    io, thread,
    time::{Duration, Instant},
};

mod telemetry;
use telemetry::{Telemetry, TelemetryError};

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

fn run_source(bind: &str, target: &str, unicast: bool, running: Arc<AtomicBool>) -> io::Result<()> {
    let socket = UdpSocket::bind(bind).expect("Failed to bind UDP socket");
    if unicast {
        socket.connect(target).expect("Failed to connect to target");
    }

    // Keep trying to open telemetry until successful or interrupted
    println!("Waiting for target to start...");
    let telemetry = loop {
        if !running.load(Ordering::SeqCst) {
            return Ok(());
        }

        match Telemetry::open() {
            Ok(telemetry) => {
                println!("Connected to target");
                println!("Memory region size: {} bytes", telemetry.size());
                break telemetry;
            }
            Err(TelemetryError::Unavailable) => {
                thread::sleep(Duration::from_secs(1));
                continue;
            }
            Err(TelemetryError::Other(e)) => {
                return Err(io::Error::other(e.to_string()));
            }
        }
    };

    let mut buf = [0u8; 64 * 1024];
    let mut start_time = std::time::Instant::now();
    let mut updates = 0;

    while running.load(Ordering::SeqCst) {
        if !telemetry.wait_for_data(200) {
            continue; // Timeout or error
        }

        // Compress the memory content
        let len = compress_to_buffer(telemetry.as_slice(), None, true, &mut buf)
            .expect("LZ4 compression failed");

        if !unicast {
            socket.send_to(&buf[..len], target).unwrap();
        } else {
            socket.send(&buf[..len]).unwrap();
        }

        updates += 1;

        if start_time.elapsed() >= Duration::from_secs(30) {
            let rate = updates as f64 / 30.0;
            println!("[source] {:.2} updates/sec", rate);
            updates = 0;
            start_time = Instant::now();
        }
    }

    Ok(())
}

fn run_target(
    bind: &str,
    unicast: bool,
    group: String,
    running: Arc<AtomicBool>,
) -> io::Result<()> {
    let socket = UdpSocket::bind(bind)?;
    println!("Target bound to {}", bind);

    if !unicast {
        let group_ip: Ipv4Addr = group.parse().expect("Invalid multicast group IP");

        let local_ip = match bind.parse::<SocketAddr>() {
            Ok(addr) => match addr.ip() {
                IpAddr::V4(ipv4) => ipv4,
                _ => panic!("Only IPv4 is supported for multicast"),
            },
            Err(_) => Ipv4Addr::UNSPECIFIED,
        };

        socket
            .join_multicast_v4(&group_ip, &local_ip)
            .expect("Failed to join multicast group");

        println!("Joined multicast group: {}", group_ip);
    }

    let mut rcv_buf = [0u8; 64 * 1024];

    // Create telemetry mapping
    let mapping_size = 32 * 1024 * 1024; // 32 MB
    let mut telemetry = Telemetry::create(mapping_size)
        .map_err(|e| io::Error::other(format!("Failed to create telemetry: {}", e)))?;

    println!("Memory-mapped file and data-valid event created.");

    let mut start_time = std::time::Instant::now();
    let mut updates = 0;

    while running.load(Ordering::SeqCst) {
        let (amt, _) = socket.recv_from(&mut rcv_buf)?;
        decompress_to_buffer(&rcv_buf[0..amt], None, telemetry.as_slice_mut())
            .expect("LZ4 decompression failed");

        telemetry
            .signal_data_ready()
            .map_err(|e| io::Error::other(format!("Failed to signal data ready: {}", e)))?;

        updates += 1;

        if start_time.elapsed() >= Duration::from_secs(30) {
            let rate = updates as f64 / 30.0;
            println!("[target] {:.2} updates/sec", rate);
            updates = 0;
            start_time = Instant::now();
        }
    }

    Ok(())
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
        } => run_source(&bind, &target, unicast, running).map_err(|e| {
            eprintln!("Error in source: {}", e);
            io::Error::other("Source error")
        }),

        Mode::Target {
            bind,
            group,
            unicast,
        } => run_target(&bind, unicast, group, running),
    }
}
