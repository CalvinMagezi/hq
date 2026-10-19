//! The root applier's entry point; see `hq_net::applier`. Linux only: on any other target this is a
//! stub, so no privileged code is compiled into a build for macOS, Android or Windows.

#[cfg(target_os = "linux")]
fn main() {
    std::process::exit(hq_net::applier::main());
}

#[cfg(not(target_os = "linux"))]
fn main() {
    eprintln!("hq-net-apply runs on Linux only");
    std::process::exit(2);
}
