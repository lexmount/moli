use std::process::Command;
use vergen_gitcl::{Emitter, Gitcl};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("cargo::rustc-check-cfg=cfg(moli_jemalloc_target)");
    let target_os = std::env::var("CARGO_CFG_TARGET_OS")?;
    let target_env = std::env::var("CARGO_CFG_TARGET_ENV")?;
    let target_arch = std::env::var("CARGO_CFG_TARGET_ARCH")?;
    let uses_partition_alloc =
        (target_os == "linux" && target_env == "gnu" && target_arch == "x86_64")
            || (target_os == "macos" && matches!(target_arch.as_str(), "x86_64" | "aarch64"));
    if target_os != "windows" && !uses_partition_alloc {
        println!("cargo::rustc-cfg=moli_jemalloc_target");
    }

    let git = Gitcl::builder().sha(false).build();
    Emitter::default()
        .fail_on_error()
        .add_instructions(&git)?
        .emit()?;

    // vergen-gitcl 10.0.2 resolves `--git-dir` to the per-worktree directory,
    // then looks for the branch ref below it. Linked worktrees keep branch refs
    // in the common Git directory instead, so make that dependency explicit.
    let symbolic_ref = Command::new("git")
        .args(["symbolic-ref", "--quiet", "HEAD"])
        .output()?;
    if symbolic_ref.status.success() {
        let symbolic_ref = String::from_utf8(symbolic_ref.stdout)?;
        let ref_path = Command::new("git")
            .args(["rev-parse", "--git-path", symbolic_ref.trim()])
            .output()?;
        if !ref_path.status.success() {
            return Err("git rev-parse --git-path failed".into());
        }
        println!(
            "cargo:rerun-if-changed={}",
            String::from_utf8(ref_path.stdout)?.trim()
        );
    }
    Ok(())
}
