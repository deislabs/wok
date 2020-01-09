# Examples

This directory contains some of the example workloads that can be run using wok.

## helloworld-demo

A headless demonstration that can be compiled with the `wasm32-wasi` target. It displays a friendly message every few seconds.

First, install [wasmtime](https://github.com/bytecodealliance/wasmtime). Then,

```
cargo build --target wasm32-wasi
wasmtime ./target/wasm32-wasi/debug/helloworld-demo.wasm
```

If it worked, you should be greeted with a friendly message every few seconds.

## wasm32-wasi-demo

A sample WebAssembly Module that can be compiled with the `wasm32-wasi` target. It listens for requests on port 8080 and responds with a friendly message.

NOTE: this demo was built in preparation for [socket support](https://github.com/bytecodealliance/wasmtime/pull/539) landing in wasmtime. It currently does not work (but it likely will in the future!)

First, install [wasmtime](https://github.com/bytecodealliance/wasmtime). Then,

```
cargo build --target wasm32-wasi
wasmtime --port 8080 ./target/wasm32-wasi/debug/wasm32-wasi-demo.wasm
```

Then, open a browser to http://127.0.0.1:8080. If it worked, you should be greeted with a friendly message.
