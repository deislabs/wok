
run:
    RUST_LOG=wok=info cargo run --bin wok-server

build:
    cargo build

install:
    RUSTFLAGS=-Awarnings cargo install -f --path .

client:
    cargo run --bin wok-client