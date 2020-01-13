
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

crictl-version:
    @echo "Requires sudo to execute on protected socket"
    sudo crictl -c ./crictl.yaml version

socat:
    @echo "Requires sudo to execute on protected socket"
    sudo socat -v UNIX-LISTEN:/var/run/wok.sock,fork TCP:127.0.0.1:50051