fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure().compile(
        &["proto/runtime/v1alpha2/api.proto"],
        &[
            "proto",
            "proto/runtime/v1alpha2/",
            //"proto/github.com/gogo/protobuf/gogoproto/",
        ],
    )?;

    println!("cargo:rustc-link-search=native={}", "./target");
    println!("cargo:rustc-link-lib=static={}", "wasm2oci");

    #[cfg(target_os = "macos")]
    {
        println!("cargo:rustc-flags=-l framework=CoreFoundation -l framework=Security");
    }

    Ok(())
}
