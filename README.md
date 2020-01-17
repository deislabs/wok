# WOK: WebAssembly On Kubernetes

![](https://github.com/deislabs/wok/workflows/Build%20and%20Test/badge.svg)

WOK is a CRI implementation that runs WASM on Kubernetes.

This is part of Project Arecibo, our ongoing effort to create a Cloud Native ecosystem for WebAssembly.

## Getting Started

Prerequisites:

- `git`
- `go` and `dep`
- `cargo`
- [`just`](https://github.com/casey/just)
- `clippy` (`rustup component add clippy`)
- `crictl` and `critest` for integration testing

Ensure you clone this project in your `GOPATH`.  The environment variable `GO111MODULE` is unset by the bootstrap build script.  If you build manually, ensure that you `unset GO111MODULE` first.

Pick up some work from the project board: https://github.com/deislabs/wok/projects/1

The easiest way to run this code is to install and use [Just](https://github.com/casey/just), a make-like tool with some super handy features.

Open two terminals: one for the client, and one for the server.

Terminal 1:

```
$ just bootstrap
$ just run
```

Terminal 2:

```
$ just server-version
```

To build binaries of both the client and the server, run `just build`.

(If you would prefer to run raw Cargo commands, you can look at the `justfile` for examples)

## Testing the Server

Kubernetes provides a CRI conformance tool called `critest` and a client called `crictl`. You can use these to work with Wok.

To get started, install the tools according to one of the methods described in the [cri-tools project documentation](https://github.com/kubernetes-sigs/cri-tools).

To run a simple test, start your server in one terminal (`just run`), and then open a new terminal and run `just server-version`. To run the full integration test suite, execute `just test-integration` in its own terminal.

## References:

- Tutorial for Tonic: https://github.com/hyperium/tonic/blob/master/examples/helloworld-tutorial.md
- CRI: https://github.com/kubernetes/cri-api
- Krustlet: https://github.com/deislabs/krustlet/

## Code of Conduct

This project has adopted the [Microsoft Open Source Code of Conduct](https://opensource.microsoft.com/codeofconduct/). For more information see the [Code of Conduct FAQ](https://opensource.microsoft.com/codeofconduct/faq/) or contact [opencode@microsoft.com](mailto:opencode@microsoft.com) with any additional questions or comments.
