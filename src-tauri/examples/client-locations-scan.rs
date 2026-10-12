//! Metadata-only scan benchmark: no client config reads or writes.
//! cargo run --example client-locations-scan --no-default-features --features test-support,search-static -- /path/to/home
fn main() {
    let home = std::env::args_os()
        .nth(1)
        .map(std::path::PathBuf::from)
        .or_else(dirs::home_dir)
        .expect("home directory");
    let (entries, before, after) = conduit_lib::clients::measure_client_location_scan(&home, 50);
    println!(
        "entries={entries} iterations=50 profile=debug before_ms={before:.3} after_ms={after:.3}"
    );
}
