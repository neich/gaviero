// Tier W1 / PR-6: embed a Windows application manifest declaring
// `longPathAware`, so Win32 file APIs accept paths beyond MAX_PATH when
// the `LongPathsEnabled` registry policy is also on. UTF-8 active code
// page keeps `OsStr` conversions lossless. No effect on other targets.
use embed_manifest::manifest::{ActiveCodePage, Setting};
use embed_manifest::{embed_manifest, new_manifest};

fn main() {
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let manifest = new_manifest("Gaviero.Tui")
            .long_path_aware(Setting::Enabled)
            .active_code_page(ActiveCodePage::Utf8);
        embed_manifest(manifest).expect("unable to embed manifest file");
    }
    println!("cargo:rerun-if-changed=build.rs");
}
