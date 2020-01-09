# `libwasm2oci`

This is a library built using [`github.com/engineerd/wasm-to-oci`](https://github.com/engineerd/wasm-to-oci) and [`github.com/deislabs/oras`](https://github.com/deislabs/oras) to distribute WebAssembly modules using OCI registries (tested usuing Docker Distribution 2.7+ and Aure Container Registries).

`wok` uses `libwasm2oci` as a static library, linked at compilation time. This ensures that the projects using this library can be distributed as a single binary. Because the resulting library `libwasm2oci.a` is platform dependent, it needs to be compiled on the same platform as the project using it.

### Building

In the root of this repository:

```
$ just bootstrap
CGO_ENABLED=1 go build -buildmode=c-archive -o target/libwasm2oci.a libwasm2oci/libwasm2oci.go
```

This will create `libwasm2oci.a` in the `target/` directory, which will be linked by Cargo (see `build.rs`).
