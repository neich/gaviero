// Tier W1 / PR-6: embed a Windows application manifest declaring
// `longPathAware`, so Win32 file APIs accept paths beyond MAX_PATH when
// the `LongPathsEnabled` registry policy is also on. UTF-8 active code
// page keeps `OsStr` conversions lossless. No effect on other targets.
use embed_manifest::manifest::{ActiveCodePage, Setting};
use embed_manifest::{embed_manifest, new_manifest};

fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let manifest = new_manifest("Gaviero.Cli")
            .long_path_aware(Setting::Enabled)
            .active_code_page(ActiveCodePage::Utf8);
        embed_manifest(manifest).expect("unable to embed manifest file");
    }
    // Reserve an 8 MiB main-thread stack, matching the Linux default. The
    // Windows default is 1 MiB, and an unoptimized build of this binary's
    // clap-derived `Cli` plus its async `main` sits right at that limit: adding
    // a handful of flags overflowed the stack before any mode ran.
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc") {
        println!("cargo:rustc-link-arg-bins=/STACK:8388608");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
