//! `hdm`, Herb's Download Manager's command line and MCP server (`hd_core::cli`).

fn main() {
    std::process::exit(hd_core::cli::main(std::env::args().skip(1).collect()));
}
