# celobus-proto

The [Celobus](https://celobus.com) plugin contract as Rust code: the protobuf messages and gRPC services of
`plugin.proto`, generated with tonic and prost. The contract is compiled in Rust at build time; no `protoc` is needed.
Plugins use it through [`celobus-sdk`](https://crates.io/crates/celobus-sdk).

Source-available under the PolyForm Shield License 1.0.0 (`LICENSE`).
