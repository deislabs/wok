# WOK: WebAssembly On Kubernetes

![](https://github.com/deislabs/wok/workflows/Build%20and%20Test/badge.svg)

WOK is a CRI implementation that runs WASM on Kubernetes.

This is part of Project Arecibo, our ongoing effort to create a Cloud Native ecosystem for WebAssembly.

IF YOU THINK THIS README SUCKS, THEN [ISSUE #5](https://github.com/deislabs/wok/issues/5) IS FOR YOU!

## Getting Started

Pick up some work from the project board: https://github.com/deislabs/wok/projects/1

Open two terminals: one for the client, and one for the server.

Terminal 1:

```
$ cargo run --bin wok-server
```

Terminal 2:

```
$ cargo run --bin wok-client
```

## References:

- Tutorial for Tonic: https://github.com/hyperium/tonic/blob/master/examples/helloworld-tutorial.md
- CRI: https://github.com/kubernetes/cri-api
- Krustlet: https://github.com/deislabs/krustlet/

## Code of Conduct

This project has adopted the [Microsoft Open Source Code of Conduct](https://opensource.microsoft.com/codeofconduct/). For more information see the [Code of Conduct FAQ](https://opensource.microsoft.com/codeofconduct/faq/) or contact [opencode@microsoft.com](mailto:opencode@microsoft.com) with any additional questions or comments.
