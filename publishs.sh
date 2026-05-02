#!/bin/bash

cargo publish --registry crates-io --all-features --manifest-path rmqtt/Cargo.toml

sleep 15

cargo publish --registry crates-io --all-features --manifest-path rmqtt-plugins/rmqtt-acl/Cargo.toml
cargo publish --registry crates-io --all-features --manifest-path rmqtt-plugins/rmqtt-retainer/Cargo.toml
cargo publish --registry crates-io --all-features --manifest-path rmqtt-plugins/rmqtt-http-api/Cargo.toml
cargo publish --registry crates-io --all-features --manifest-path rmqtt-plugins/rmqtt-counter/Cargo.toml
cargo publish --registry crates-io --all-features --manifest-path rmqtt-plugins/rmqtt-auth-http/Cargo.toml
cargo publish --registry crates-io --all-features --manifest-path rmqtt-plugins/rmqtt-auth-jwt/Cargo.toml
cargo publish --registry crates-io --all-features --manifest-path rmqtt-plugins/rmqtt-auto-subscription/Cargo.toml
cargo publish --registry crates-io --all-features --manifest-path rmqtt-plugins/rmqtt-bridge-egress-kafka/Cargo.toml
cargo publish --registry crates-io --all-features --manifest-path rmqtt-plugins/rmqtt-bridge-ingress-kafka/Cargo.toml
cargo publish --registry crates-io --all-features --manifest-path rmqtt-plugins/rmqtt-bridge-egress-mqtt/Cargo.toml
cargo publish --registry crates-io --all-features --manifest-path rmqtt-plugins/rmqtt-bridge-ingress-mqtt/Cargo.toml
cargo publish --registry crates-io --all-features --manifest-path rmqtt-plugins/rmqtt-bridge-ingress-pulsar/Cargo.toml
cargo publish --registry crates-io --all-features --manifest-path rmqtt-plugins/rmqtt-bridge-egress-pulsar/Cargo.toml
cargo publish --registry crates-io --all-features --manifest-path rmqtt-plugins/rmqtt-bridge-ingress-nats/Cargo.toml
cargo publish --registry crates-io --all-features --manifest-path rmqtt-plugins/rmqtt-bridge-egress-nats/Cargo.toml
cargo publish --registry crates-io --all-features --manifest-path rmqtt-plugins/rmqtt-bridge-egress-reductstore/Cargo.toml
cargo publish --registry crates-io --all-features --manifest-path rmqtt-plugins/rmqtt-message-storage/Cargo.toml
cargo publish --registry crates-io --all-features --manifest-path rmqtt-plugins/rmqtt-session-storage/Cargo.toml
cargo publish --registry crates-io --all-features --manifest-path rmqtt-plugins/rmqtt-sys-topic/Cargo.toml
cargo publish --registry crates-io --all-features --manifest-path rmqtt-plugins/rmqtt-topic-rewrite/Cargo.toml
cargo publish --registry crates-io --all-features --manifest-path rmqtt-plugins/rmqtt-web-hook/Cargo.toml
cargo publish --registry crates-io --all-features --manifest-path rmqtt-plugins/rmqtt-cluster-raft/Cargo.toml
cargo publish --registry crates-io --all-features --manifest-path rmqtt-plugins/rmqtt-cluster-broadcast/Cargo.toml
cargo publish --registry crates-io --all-features --manifest-path rmqtt-plugins/rmqtt-p2p-messaging/Cargo.toml

sleep 15

cargo publish --registry crates-io --all-features --manifest-path rmqtt-plugins/Cargo.toml

sleep 5

cargo publish --registry crates-io --all-features --manifest-path rmqtt-bin/Cargo.toml

# cargo publish --registry crates-io --all-features --manifest-path rmqtt-utils/Cargo.toml
# cargo publish --registry crates-io --all-features --manifest-path rmqtt-codec/Cargo.toml
# cargo publish --registry crates-io --all-features --manifest-path rmqtt-net/Cargo.toml
# cargo publish --registry crates-io --all-features --manifest-path rmqtt-conf/Cargo.toml

