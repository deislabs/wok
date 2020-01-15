

# Override the RUST_LOG value with custom log levels.
# When debugging gRPC, it may be useful to do `just log_level=debug run`
log_level := "wok=info"

# Location of your 'crictl.yaml' config file
crictl_yaml := "./crictl.yaml"

# Name of the socket the code starts. Only override this if you
# are sure of what you are doing. Otherwise, edit 'crictl.yaml'
# or the code.
wok_sock := "/tmp/wok.sock"

run:
    RUST_LOG={{log_level}} cargo run

build:
    cargo build

install:
    RUSTFLAGS=-Awarnings cargo install -f --path .

test:
    cargo test

bootstrap:
    cd libwasm2oci && dep ensure -v
    GO111MODULE= CGO_ENABLED=1 go build -buildmode=c-archive -o target/libwasm2oci.a libwasm2oci/libwasm2oci.go

# A quick test to make sure the server is executing.
server-version:
    crictl -c {{crictl_yaml}} version

test-integration:
    critest --runtime-endpoint {{wok_sock}}