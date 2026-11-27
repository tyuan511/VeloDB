use std::env;
use velodb::adapter::create_adapter;
use velodb::config::load_connections;
use velodb::mcp::run_stdio_mcp;

#[tokio::main]
async fn main() {
    let args: Vec<String> = env::args().collect();
    let mut conn_name: Option<String> = None;
    let mut read_only = false;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--connection" if i + 1 < args.len() => {
                conn_name = Some(args[i + 1].clone());
                i += 2;
            }
            "--read-only" => {
                read_only = true;
                i += 1;
            }
            _ => i += 1,
        }
    }

    let conns = load_connections();
    let cfg = if let Some(name) = conn_name {
        conns
            .into_iter()
            .find(|c| c.name == name || c.id == name)
            .expect("Connection not found in ~/.config/velodb/connections.json")
    } else {
        conns
            .into_iter()
            .next()
            .expect("No connections configured in ~/.config/velodb/connections.json")
    };

    let adapter = create_adapter(cfg).expect("Failed to create database adapter");
    if let Err(e) = run_stdio_mcp(adapter, read_only).await {
        eprintln!("velodb-mcp error: {e}");
        std::process::exit(1);
    }
}
