//! Inspect a copied savings directory without reading a registry or keychain.
fn main() {
    let dir = std::env::args_os().nth(1).expect("usage: savings-report DATA_DIR");
    std::env::set_var("TOOLPORT_DATA_DIR", dir);
    println!("{}", conduit_lib::savings::try_summary().expect("read copied savings"));
}
