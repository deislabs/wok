critools_version := "1.17.0"

run:
    RUST_LOG=wok=info cargo run --bin wok-server

build:
    cargo build

install:
    RUSTFLAGS=-Awarnings cargo install -f --path .

client:
    cargo run --bin wok-client

test:
    cargo test

bootstrap:
    cd libwasm2oci && dep ensure -v
    GO111MODULE= CGO_ENABLED=1 go build -buildmode=c-archive -o target/libwasm2oci.a libwasm2oci/libwasm2oci.go

critools:
    curl -sSL https://github.com/kubernetes-sigs/cri-tools/releases/download/v{{critools_version}}/crictl-v{{critools_version}}-linux-amd64.tar.gz | tar xzf -
    curl -sSL https://github.com/kubernetes-sigs/cri-tools/releases/download/v{{critools_version}}/critest-v{{critools_version}}-linux-amd64.tar.gz | tar xzf -
    install crictl critest /usr/local/bin
    rm crictl critest
