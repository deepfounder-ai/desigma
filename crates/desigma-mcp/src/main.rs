// Claude launches this process with its own stdin and stdout. A console subsystem would open a
// command window on Windows; a windowed process keeps those pipes and shows nothing.
#![cfg_attr(all(windows, not(debug_assertions), not(test)), windows_subsystem = "windows")]

use desigma_mcp::server::DesigmaServer;
use rmcp::{transport::stdio, ServiceExt};

// rmcp spawns one task per request. On a single-threaded runtime those tasks run in the
// order the requests arrived, and each tool call holds the session for its whole (synchronous)
// run, so calls execute and answer strictly in order (#31): a create followed by a read in the
// same burst sees the created layer. The session is one document, so nothing is lost.
#[tokio::main(flavor = "current_thread")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut args = std::env::args_os().skip(1);
    let root = match (args.next(), args.next(), args.next()) {
        (Some(flag), Some(path), None) if flag == "--workspace" => std::path::PathBuf::from(path),
        (Some(flag), None, None) if flag == "--help" => {
            eprintln!("desigma-mcp --workspace DIRECTORY\nLocal MCP server over stdio. Edits the document open in Desigma when its agent bridge is on; otherwise\nworks on an isolated file session that is kept until explicitly saved to a new file.");
            return Ok(());
        }
        _ => return Err("Usage: desigma-mcp --workspace DIRECTORY".into()),
    };
    let service = DesigmaServer::new(&root)?.serve(stdio()).await?;
    service.waiting().await?;
    Ok(())
}
