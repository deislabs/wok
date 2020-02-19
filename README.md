# WOK: WebAssembly On Kubernetes

[![Build
Status](https://github.com/deislabs/wok/workflows/Build%20and%20Test/badge.svg?branch=master)](https://github.com/deislabs/wok/actions?query=workflow%3A%22Build+and+Test%22)

WOK is a CRI implementation that runs WASM on Kubernetes.

This is part of Project Arecibo, our ongoing effort to create a Cloud Native
ecosystem for WebAssembly.

- [WOK: WebAssembly On Kubernetes](#wok-webassembly-on-kubernetes)
  - [Getting Started](#getting-started)
    - [Using with Kubernetes](#using-with-kubernetes)
      - [Minikube](#minikube)
      - [Kind](#kind)
      - [AKS](#aks)
      - [EKS](#eks)
      - [GKE](#gke)
      - [k3s on Raspberry Pi](#k3s-on-raspberry-pi)
      - [Other ARM targets](#other-arm-targets)
    - [Creating a Pod that uses WASM](#creating-a-pod-that-uses-wasm)
  - [Contributing](#contributing)
    - [Prerequisites](#prerequisites)
      - [Wait, why is there random Go in here?](#wait-why-is-there-random-go-in-here)
    - [Testing the Server](#testing-the-server)
  - [References](#references)
  - [Roadmap](#roadmap)
  - [Code of Conduct](#code-of-conduct)

## Getting Started

:rotating_light: :rotating_light: :rotating_light: **NOTE** This is currently in
an R&D state and we will continue iterating on it rapidly. DO NOT count on any
backward compatibility for now and know that there are some missing features due
to the nature of being on the bleeding edge of WASM :rotating_light:
:rotating_light: :rotating_light:

Currently, you will need to build WOK on your own to use it. We will hopefully
have official releases soon that can be downloaded from the [releases
page](https://github.com/deislabs/wok/releases).

### Using with Kubernetes
:construction: :construction: This section is currently under construction. We
will be updating instructions here as we test on each of these systems. However,
these are currently our targets. :construction: :construction:

#### Minikube

#### Kind

#### AKS

#### EKS

#### GKE

#### k3s on Raspberry Pi

#### Other ARM targets

### Creating a Pod that uses WASM
:construction: Currently under construction. Check back later :construction:

## Contributing
This section details how to get started developing on WOK. For the full
contributing process, see the [Contributing Guide](./CONTRIBUTING.md)

To get started on the some work, you can pick up tasks from the [project
board](https://github.com/deislabs/wok/projects/1).

### Prerequisites

- `git`
- `go` and `dep`
- `cargo`
- [`just`](https://github.com/casey/just)
- `clippy` (`rustup component add clippy`)
- `bindgen` (`cargo install bindgen`)
- `crictl` and `critest` for integration testing

Ensure you clone this project in your `GOPATH`.  The environment variable
`GO111MODULE` is unset by the bootstrap build script.  If you build manually,
ensure that you `unset GO111MODULE` first.

The easiest way to run this code is to install and use
[Just](https://github.com/casey/just), a make-like tool with some super handy
features.

#### Wait, why is there random Go in here?
To get things working without reinventing the wheel, we pulled in the wasm2oci
code from Go so we don't have to write that all over again. Eventually we will
try to make that part purely Rust, but for now it is a sufficient middle ground
for what we need here.

### Testing the Server

Kubernetes provides a CRI conformance tool called `critest` and a client called
`crictl`. You can use these to work with Wok.

To get started, install the tools according to one of the methods described in
the [cri-tools project
documentation](https://github.com/kubernetes-sigs/cri-tools).

To run a simple test, open two terminals: one for the client, and one for the
server.

Terminal 1:

```
$ just bootstrap
$ just run
```

Terminal 2:

```
$ just server-version
```

To build binaries for the server, run `just build`.

(If you would prefer to run raw Cargo commands, you can look at the `justfile`
for examples)

## References

- Tutorial for Tonic:
  https://github.com/hyperium/tonic/blob/master/examples/helloworld-tutorial.md
- CRI: https://github.com/kubernetes/cri-api
- Krustlet: https://github.com/deislabs/krustlet/

## Roadmap
For a full list of tasks, please visit the [project
board](https://github.com/deislabs/wok/projects/1). However, below is a general
map of where we plan on taking this:

- Making sure `critest` passes :white_check_mark:
- Writing instructions for running on various Kubernetes distributions
  :black_nib:
- Follow changes in the WASI spec (better stopping of running processes, full
  networking support, etc) :speedboat: :dash:

## Code of Conduct

This project has adopted the [Microsoft Open Source Code of
Conduct](https://opensource.microsoft.com/codeofconduct/). For more information
see the [Code of Conduct
FAQ](https://opensource.microsoft.com/codeofconduct/faq/) or contact
[opencode@microsoft.com](mailto:opencode@microsoft.com) with any additional
questions or comments.
