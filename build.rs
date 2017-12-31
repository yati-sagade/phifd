extern crate protoc_rust;

fn main() {
    protoc_rust::run(protoc_rust::Args {
        out_dir: "src/proto",
        input: &["proto/msg.proto"],
        includes: &["proto"],
    }).expect("Protoc failed");
}
