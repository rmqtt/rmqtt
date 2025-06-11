# rmqtt-utils

[![crates.io page](https://img.shields.io/crates/v/rmqtt-utils.svg)](https://crates.io/crates/rmqtt-utils/0.1.2)
[![docs.rs page](https://docs.rs/rmqtt-utils/badge.svg)](https://docs.rs/rmqtt-utils/0.1.2/rmqtt_utils)


**`rmqtt-utils`** is a lightweight Rust utility crate designed to support common system-level operations with a focus on performance, reliability, and developer ergonomics. It is especially useful in distributed systems like MQTT brokers but is general-purpose enough for any Rust project.

## ✨ Features

- **📦 Byte Size Parsing & Formatting**  
  Handle human-readable byte strings like `2G512M` via the `Bytesize` type, with full serde support.

- **⏳ Flexible Duration Parsing**  
  Convert strings like `1h30m15s` or `2w3d12h` into `std::time::Duration` objects.

- **🕒 Timestamp Utilities**  
  Retrieve and format current timestamps with second or millisecond precision.

- **🌐 Cluster Node Addressing**  
  Parse and represent cluster node addresses in the format `ID@host:port` via `NodeAddr`.

- **📈 Thread-Safe Counter**  
  Mergeable `Counter` type with internal atomic implementation, useful for metrics collection.

- **🧰 Serde Helpers**  
  Custom deserialization logic for durations, socket addresses, and other types.

---

## 🧩 Key Components

| Component         | Description                                                                 |
|-------------------|-----------------------------------------------------------------------------|
| `Bytesize`        | Parses `2G512M`-style strings and converts them to byte values              |
| `to_duration`     | Parses complex duration strings like `2w3d12h` into `Duration`              |
| `NodeAddr`        | Parses and stores cluster node IDs and addresses (`1@host:port`)            |
| `Counter`         | Thread-safe integer counter with merge support                              |
| `timestamp_secs`  | Returns current UNIX timestamp in seconds                                   |
| `format_timestamp_now` | Returns the current UTC timestamp as a formatted string               |

---

## 🛠 Usage Examples

```rust
use rmqtt_utils::{
    Bytesize, NodeAddr,
    to_bytesize, to_duration,
    timestamp_secs, format_timestamp_now
};

// Byte size parsing
let size = Bytesize::try_from("2G512M").unwrap();
assert_eq!(size.as_usize(), 2_684_354_560);

// Duration parsing
let duration = to_duration("1h30m15s").unwrap();
assert_eq!(duration.as_secs(), 5415);

// Node address parsing
let node: NodeAddr = "1@mqtt-node:1883".parse().unwrap();
assert_eq!(node.id, 1);

// Current timestamp formatting
let now = format_timestamp_now();
println!("Now: {now}");
```

---

## 🛡 Safety & Reliability

- ✅ `#![deny(unsafe_code)]` enforced — no unsafe blocks allowed
- ✅ Robust error handling for parsing operations
- ✅ Cross-platform compatibility
- ✅ Accurate UTC timestamp formatting using [`chrono`](https://docs.rs/chrono)

---

## 📦 Crate Usage

Add the following to your `Cargo.toml`:

```toml
[dependencies]
rmqtt-utils = "0.1" # Replace with the latest version
```

Then, in your code:

```rust
use rmqtt_utils::{Bytesize, to_duration, NodeAddr, format_timestamp_now};
```

---

## 🔗 Related Crates

This crate is designed to be used as part of the [`rmqtt`](https://github.com/rmqtt/rmqtt) MQTT broker project, but it is modular and reusable on its own.

---



