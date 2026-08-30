use std::env;

fn main() {
    let is_release = env::var("PROFILE").as_deref() == Ok("release");
    let is_linux_gnu = env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux")
        && env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu");

    if is_release && is_linux_gnu {
        println!("cargo:rustc-link-arg-bin=moli=-Wl,-z,pack-relative-relocs");
    }
}
