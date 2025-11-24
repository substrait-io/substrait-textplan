// SPDX-License-Identifier: Apache-2.0

use prost::Message;
use prost_build::Config;
use prost_types::FileDescriptorSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::{DirEntry, WalkDir};

const PROTO_ROOT: &str = "third_party/substrait/proto";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Always generate visitor code
    // This still uses the substrait proto definitions to generate visitor code
    // but uses the substrait crate for the actual proto definitions at runtime.
    match generate_visitor_code() {
        Ok(_) => println!("cargo:warning=Visitor code generation completed successfully"),
        Err(e) => println!("cargo:warning=Visitor code generation failed: {}", e),
    }

    Ok(())
}

fn build_proto_descriptor(out_dir: &Path) -> Result<FileDescriptorSet, Box<dyn std::error::Error>> {
    let toplevel_dir = PathBuf::from("..").canonicalize().unwrap();
    let proto_dir = toplevel_dir.join(PROTO_ROOT);

    let protos = WalkDir::new(&proto_dir)
        .into_iter()
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_file() || entry.file_type().is_symlink())
        .filter(|entry| {
            entry
                .path()
                .extension()
                .filter(|&extension| extension == "proto")
                .is_some()
        })
        .map(DirEntry::into_path)
        .inspect(|entry| {
            println!("cargo:rerun-if-changed={}", entry.display());
        })
        .collect::<Vec<_>>();

    let descriptor_path = out_dir.join("proto_descriptor.bin");
    let mut cfg = Config::new();
    cfg.file_descriptor_set_path(&descriptor_path);
    cfg.compile_protos(&protos, &[proto_dir.to_str().unwrap()])?;

    let file_descriptor_contents = std::fs::read(&descriptor_path)?;
    let buf = bytes::Bytes::from(file_descriptor_contents);

    let file_descriptor_set = prost_types::FileDescriptorSet::decode(buf).unwrap();
    Ok(file_descriptor_set)
}

mod visitor_generator {
    include!("../../build-tools/visitor_generator.rs");
}

/// Generate visitor code using our generator
fn generate_visitor_code() -> Result<(), Box<dyn std::error::Error>> {
    // Path to the output file
    let output_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    let output_path = output_dir.join("base_plan_visitor.rs");

    // Create the output directory if it doesn't exist
    fs::create_dir_all(&output_dir)?;

    println!("cargo:warning=Generating visitor code from Substrait protobuf schema");
    let proto_descriptor = build_proto_descriptor(output_dir.as_path())?;

    // Run the visitor generator directly
    println!("cargo:warning=Output path: {}", output_path.display());
    visitor_generator::generate(proto_descriptor, &output_path)?;

    Ok(())
}
