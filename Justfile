
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
    CGO_ENABLED=1 go build -buildmode=c-archive -o target/libwasm2oci.a libwasm2oci/libwasm2oci.go 
