//! Debug helper: dump a `.fig` file as JSON.
//!
//!   cargo run -p desigma-fig --example fig2json -- file.fig            # raw kiwi message
//!   cargo run -p desigma-fig --example fig2json -- file.fig --doc      # imported Document
//!   cargo run -p desigma-fig --example fig2json -- file.fig --rest     # re-exported REST JSON

fn main() {
    let mut args = std::env::args().skip(1);
    let Some(path) = args.next() else {
        eprintln!("usage: fig2json <file.fig> [--doc | --rest]");
        std::process::exit(2);
    };
    let mode = args.next();
    let bytes = match std::fs::read(&path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("cannot read {path}: {e}");
            std::process::exit(1);
        }
    };
    let out = match mode.as_deref() {
        Some("--doc") => desigma_fig::import_fig(&bytes).map(|d| d.to_json_pretty().unwrap_or_default()),
        Some("--rest") => desigma_fig::import_fig(&bytes).map(|d| desigma_fig::export_rest_json(&d).to_string()),
        _ => desigma_fig::decode_fig_to_json(&bytes).map(|v| serde_json::to_string_pretty(&v).unwrap_or_default()),
    };
    match out {
        Ok(s) => println!("{s}"),
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
}
