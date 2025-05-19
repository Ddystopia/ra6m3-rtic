use std::path::PathBuf;

fn main() {
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let memory_x = out_dir.join("memory.x");

    println!("cargo:rerun-if-changed=lm3s6965_memory.x");
    println!("cargo:rerun-if-changed=ra6m3_memory.x");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rustc-link-search={}", out_dir.display());

    if cfg!(feature = "qemu") {
        std::fs::copy("lm3s6965_memory.x", memory_x).unwrap();
    } else if cfg!(feature = "ra6m3") {
        std::fs::copy("ra6m3_memory.x", memory_x).unwrap();
    } else {
        panic!("No memory layout specified. Use --features lm3s6965 or ra6m3.");
    }
}
