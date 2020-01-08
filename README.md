# WOK: WebAssembly On Kubernetes

WOK is a CRI implementation that runs WASM on Kubernetes.

This is part of Project Arecibo, our ongoing effort to create a Cloud Native ecosystem for WebAssembly.

IF YOU THINK THIS README SUCKS, THEN [ISSUE #5](https://github.com/deislabs/wok/issues/5) IS FOR YOU!

## Getting Started

Pick up some work from the project board: https://github.com/deislabs/wok/projects/1

You can build the project with `cargo build`.

## Compiling from Source

First, [install Rust](https://www.rust-lang.org/tools/install). Then,

```
$ cargo run
```

## References:

- Tutorial for Tonic: https://github.com/hyperium/tonic/blob/master/examples/helloworld-tutorial.md
- CRI: https://github.com/kubernetes/cri-api
- Krustlet: https://github.com/deislabs/krustlet/
