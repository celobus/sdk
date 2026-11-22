//! The Celobus plugin contract: the protobuf messages and gRPC services of `proto/plugin.proto`.

pub mod v1 {
    tonic::include_proto!("celobus.v1");
}

pub use v1::*;
