fn main() {
    let src_dir = std::path::Path::new("src");

    // Ensure cargo rebuilds when the generated parser changes.
    println!("cargo:rerun-if-changed=src/parser.c");
    println!("cargo:rerun-if-changed=grammar.js");

    let mut c_config = cc::Build::new();
    c_config.std("c11");
    c_config.include(src_dir);
    c_config.file(src_dir.join("parser.c"));
    c_config.compile("tree-sitter-gaviero");
}
