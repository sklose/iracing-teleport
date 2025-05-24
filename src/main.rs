use clap::{Parser, Subcommand};
use lz4::block::{compress, decompress};
use std::io;
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::{thread, time::Duration};
use windows::{
    Win32::Foundation::*, Win32::System::Memory::*, core::*,
};

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

fn run_source(bind: &str, target: &str, multicast: bool) -> windows::core::Result<()> {
    unsafe {
        let socket = UdpSocket::bind(bind).expect("Failed to bind UDP socket");
        if !multicast {
            socket.connect(target).expect("Failed to connect to target");
        }

        // Open memory-mapped file
        let h_map = OpenFileMappingW(FILE_MAP_READ.0, FALSE, w!("Local\\IRSDKMemMapFileName"))?;
        let view = MapViewOfFile(h_map, FILE_MAP_READ, 0, 0, 0).Value as *const u8;
        if view.is_null() {
            panic!("Failed to map view of file");
        }

        // Use VirtualQuery to determine the size of the region
        let mut mem_info = MEMORY_BASIC_INFORMATION::default();
        let result = VirtualQuery(
            Some(view as *const _),
            &mut mem_info,
            std::mem::size_of::<MEMORY_BASIC_INFORMATION>(),
        );
        if result == 0 {
            panic!("VirtualQuery failed");
        }

        let size = mem_info.RegionSize;
        println!("Memory region size: {} bytes", size);

        let tick_rate = Duration::from_millis(1000 / 60); // 60Hz
        let data_slice = std::slice::from_raw_parts(view, size);

        loop {
            // Compress the memory content
            let compressed = compress(data_slice, None, true).expect("LZ4 compression failed");
            println!("Compressed size: {}", compressed.len());

            if multicast {
                socket.send_to(&compressed, target).unwrap();
            } else {
                socket.send(&compressed).unwrap();
            }

            thread::sleep(tick_rate);
        }

        // NOTE: unreachable due to loop, but you'd unmap here:
        // UnmapViewOfFile(view as _);
        // CloseHandle(h_map);
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

    let mut buf = [0; 64*1024];

    loop {
        let (amt, src) = socket.recv_from(&mut buf)?;
        let decompressed = decompress(&buf[..amt], None).expect("Decompression failed");
        println!("Received from {}: {}", src, decompressed.len());
    }
}

fn main() -> io::Result<()> {
    let cli = Cli::parse();

    match cli.mode {
        Mode::Source {
            bind,
            target,
            multicast,
        } => run_source(&bind, &target, multicast).map_err(|e| {
            eprintln!("Error in source: {}", e);
            io::Error::other("Source error")
        }),

        Mode::Target {
            bind,
            group,
            multicast,
        } => run_target(&bind, multicast, group),
    }
}
