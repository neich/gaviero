fn main() {
    use std::path::Path;
    let root = Path::new(r"C:\Users\natxm\Dropbox\work\git\gaviero");
    let canon = std::fs::canonicalize(root).unwrap();
    println!("canon_display={}", canon.display());
    println!("canon_lossy={}", canon.to_string_lossy());
    let s = canon.to_string_lossy();
    let mut hasher = sha2::Sha256::new();
    use sha2::Digest;
    hasher.update(s.as_bytes());
    let digest = hasher.finalize();
    let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
    println!("pipe=\\\\.\\pipe\\gaviero-{}", &hex[..16]);
}
