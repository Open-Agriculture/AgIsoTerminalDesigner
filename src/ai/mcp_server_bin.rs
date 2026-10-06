#[cfg(not(target_arch = "wasm32"))]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    ag_iso_terminal_designer::ai::mcp_server::run()
}

#[cfg(target_arch = "wasm32")]
fn main() {}
