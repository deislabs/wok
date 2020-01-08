fn main() -> Result<(), Box<dyn std::error::Error>> {
    tonic_build::configure().compile(
        &["proto/runtime/v1alpha2/api.proto"],
        &[
            "proto",
            "proto/runtime/v1alpha2/",
            //"proto/github.com/gogo/protobuf/gogoproto/",
        ],
    )?;
    Ok(())
}
