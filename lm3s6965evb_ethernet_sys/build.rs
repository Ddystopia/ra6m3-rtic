fn main() {
    let bindings = bindgen::Builder::default()
        .use_core()
        .header("ethernet.h")
        .header("inc/hw_types.h")
        .header("inc/hw_ethernet.h")
        .header("sysctl.h")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks::new()))
        .generate()
        .expect("Unable to generate bindings");

    let out_path = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());

    bindings
        .write_to_file(out_path.join("ethernet.rs"))
        .expect("Couldn't write bindings!");

    let mut builder = cc::Build::new();
    // --std=c99 -W -Wall -Werror -pedantic -O3 -flto
    builder
        .flag("--std=c99")
        .flag("-W")
        .flag("-Wall")
        .flag_if_supported("-Wextra")
        .flag("-Werror")
        .flag("-pedantic")
        .flag("-L.")
        .flag("-L./inc")
        .opt_level(3)
        // Not sure if we want these two. We can check the codegen later.
        // .pic(false)
        // .use_plt(false)
        .file("ethernet.c")
        .compile("lm3s6965evb_ethernet_sys");
}
