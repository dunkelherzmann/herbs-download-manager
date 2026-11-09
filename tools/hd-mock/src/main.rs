//! Serves test files for trying the app by hand.
//! Usage: hd-mock [--size MiB] [--mirrors N] [--port P] [--no-ranges] [--rate KiB/s]
//!        hd-mock --features [--port P]   files for the feature tests (tests/features.scenario) on P, P+1 and P+2
//! Prints the link and one mirror link per extra server, then runs until stopped.

use hd_mock::{MockFile, MockServer, pattern};

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let mut size_mib = 64usize;
    let mut mirrors = 2usize;
    let mut port = 8480u16;
    let mut ranges = true;
    let mut rate = None;
    let mut features = false;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut value = || args.next().and_then(|v| v.parse::<u64>().ok()).unwrap_or_else(|| usage());
        match a.as_str() {
            "--size" => size_mib = value() as usize,
            "--mirrors" => mirrors = value() as usize,
            "--port" => port = value() as u16,
            "--rate" => rate = Some(value() * 1024),
            "--no-ranges" => ranges = false,
            "--features" => features = true,
            _ => usage(),
        }
    }
    if features {
        return serve_features(port).await;
    }
    let data = pattern(size_mib * 1024 * 1024, 0);
    let mut servers = Vec::new();
    for i in 0..=mirrors {
        let server = MockServer::bind(&format!("127.0.0.1:{}", port + i as u16)).await?;
        let mut file = MockFile::new(data.clone());
        file.ranges = ranges;
        file.rate = rate;
        server.put("/test.bin", file);
        println!("{} {}", if i == 0 { "link  " } else { "mirror" }, server.url("/test.bin"));
        servers.push(server);
    }
    tokio::signal::ctrl_c().await?;
    Ok(())
}

async fn serve_features(port: u16) -> std::io::Result<()> {
    let servers = hd_mock::feature_servers(Some(port)).await?;
    for server in &servers {
        println!("{}", server.url("/"));
    }
    println!("sha256 small.bin {}", hd_mock::feature_small_sha256());
    tokio::signal::ctrl_c().await?;
    Ok(())
}

fn usage() -> ! {
    eprintln!("usage: hd-mock [--size MiB] [--mirrors N] [--port P] [--no-ranges] [--rate KiB/s]");
    std::process::exit(2)
}
