//! `hello --describe` prints the descriptor; otherwise the plugin serves.

fn main() {
    celobus_conformance_hello::plugin().serve()
}
