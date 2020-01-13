
run:
    RUST_LOG=wok=info cargo run

build:
    cargo build

install:
    RUSTFLAGS=-Awarnings cargo install -f --path .

test:
    cargo test

bootstrap:
    cd libwasm2oci && dep ensure -v
    GO111MODULE= CGO_ENABLED=1 go build -buildmode=c-archive -o target/libwasm2oci.a libwasm2oci/libwasm2oci.go
