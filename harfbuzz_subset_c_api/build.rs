fn main() {
    println!("cargo:rerun-if-env-changed=HARFRUST_C_LIB_DIR");
    if let Some(directory) = std::env::var_os("HARFRUST_C_LIB_DIR") {
        println!(
            "cargo:rustc-link-search=native={}",
            directory.to_string_lossy()
        );
        println!("cargo:rustc-link-lib=dylib=harfrust_c");
    } else if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("macos") {
        // Like Linux, allow the core C library to be supplied by the consumer.
        println!("cargo:rustc-link-arg-cdylib=-Wl,-undefined,dynamic_lookup");
    }
}
