use clap::{Parser, Subcommand};
use lz4::block::{compress, decompress};
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::{thread, time::Duration};

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
        /// Local bind address (e.g., 127.0.0.1:8080)
        #[arg(long, default_value = "127.0.0.1:0")]
        bind: String,

        /// Target address to send data to (e.g., 127.0.0.1:8081)
        #[arg(long, default_value = "127.0.0.1:8081")]
        target: String,

        /// Enable multicast mode
        #[arg(long)]
        multicast: bool,
    },

    /// Run as the target (receives compressed data)
    Target {
        /// Address to bind to for receiving (e.g., 127.0.0.1:8081)
        #[arg(long, default_value = "127.0.0.1:8081")]
        bind: String,

        /// Multicast group to join (required if --multicast is set)
        #[arg(long)]
        group: Option<String>,

        /// Enable multicast mode
        #[arg(long)]
        multicast: bool,
    },
}

fn run_source(bind: &str, target: &str, multicast: bool) -> io::Result<()> {
    let socket = UdpSocket::bind(bind)?;
    if multicast {
        println!("Source in MULTICAST mode -> {}", target);
    } else {
        println!("Source in UNICAST mode -> {}", target);
        socket.connect(target)?;
    }

    let tick_rate = Duration::from_millis(1000 / 60); // 60Hz

    loop {
        let msg = format!("Data from source at {:?}", std::time::Instant::now());
        let compressed = compress(msg.as_bytes(), None, true).expect("Compression failed");

        if multicast {
            socket.send_to(&compressed, target)?;
        } else {
            socket.send(&compressed)?;
        }

        thread::sleep(tick_rate);
    }
}

fn run_target(bind: &str, multicast: bool, group: Option<String>) -> io::Result<()> {
    let socket = UdpSocket::bind(bind)?;
    println!("Target bound to {}", bind);

    if multicast {
        let group_ip: Ipv4Addr = group
            .expect("Multicast group required with --multicast")
            .parse()
            .expect("Invalid multicast group IP");

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

    let mut buf = [0; 2048];

    loop {
        let (amt, src) = socket.recv_from(&mut buf)?;
        let decompressed = decompress(&buf[..amt], None).expect("Decompression failed");
        let msg = String::from_utf8_lossy(&decompressed);
        println!("Received from {}: {}", src, msg);
    }
}

fn main() -> io::Result<()> {
    let cli = Cli::parse();

    match cli.mode {
        Mode::Source {
            bind,
            target,
            multicast,
        } => run_source(&bind, &target, multicast),

        Mode::Target {
            bind,
            group,
            multicast,
        } => run_target(&bind, multicast, group),
    }
}
