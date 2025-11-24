// SPDX-License-Identifier: Apache-2.0

use prost::{bytes, Message};
use prost_build::Config;
use prost_types::FileDescriptorSet;
use std::env;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use walkdir::{DirEntry, WalkDir};

const PROTO_ROOT: &str = "third_party/substrait/proto";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Tell Cargo to re-run this build script if the grammars or visitor generator change
    println!("cargo:rerun-if-changed=third_party/substrait/proto"); // Still needed for visitor generator
    println!("cargo:rerun-if-changed=grammar");
    println!("cargo:rerun-if-changed=build-tools/visitor_generator.rs");
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=GENERATE_ANTLR");

    // Configure parser to use ANTLR4
    println!("cargo:rustc-cfg=use_antlr4");

    // Generate ANTLR code if requested
    // You can use this by setting the GENERATE_ANTLR env var:
    // GENERATE_ANTLR=true cargo build
    if env::var("GENERATE_ANTLR").is_ok() {
        match generate_antlr_code() {
            Ok(_) => println!("cargo:warning=ANTLR code generation completed successfully"),
            Err(e) => println!("cargo:warning=ANTLR code generation failed: {}", e),
        }
    }

    // Always generate visitor code
    // This still uses the substrait proto definitions to generate visitor code
    // but uses the substrait crate for the actual proto definitions at runtime.
    match generate_visitor_code() {
        Ok(_) => println!("cargo:warning=Visitor code generation completed successfully"),
        Err(e) => println!("cargo:warning=Visitor code generation failed: {}", e),
    }

    Ok(())
}

/// Generate ANTLR code using the antlr4 tool with Rust support.
///
/// Note: You must download the special ANTLR4 JAR with Rust support from:
/// https://github.com/rrevenantt/antlr4rust/releases/download/antlr4-4.8-2-Rust0.3.0-beta/antlr4-4.8-2-SNAPSHOT-complete.jar
///
/// Regular ANTLR4 JAR files DO NOT support generating Rust code!
fn generate_antlr_code() -> Result<(), Box<dyn std::error::Error>> {
    // Path to your ANTLR grammar files
    let grammar_dir = PathBuf::from("grammar");

    // Path to output the generated code
    let output_dir = PathBuf::from("src/textplan/parser/antlr");

    // Create the output directory if it doesn't exist
    fs::create_dir_all(&output_dir)?;

    // Path to the ANTLR4 jar file with Rust support
    // This must be the special version from antlr4rust repo!
    let antlr_jar = env::var("ANTLR_JAR").ok().or_else(|| {
        // Try to find the JAR in the build-tools directory
        let build_tools_jar = PathBuf::from("build-tools/antlr4rust.jar");
        if build_tools_jar.exists() {
            println!("cargo:warning=Using ANTLR4 JAR found in build-tools directory");
            return Some(build_tools_jar.to_string_lossy().to_string());
        }
        None
    }).unwrap_or_else(|| {
        println!("cargo:warning=ANTLR_JAR environment variable not set!");
        println!("cargo:warning=You must download the special ANTLR4 JAR with Rust support from:");
        println!("cargo:warning=https://github.com/rrevenantt/antlr4rust/releases/download/antlr4-4.8-2-Rust0.3.0-beta/antlr4-4.8-2-SNAPSHOT-complete.jar");
        println!("cargo:warning=Regular ANTLR4 JAR files DO NOT support generating Rust code!");
        println!("cargo:warning=Using default 'antlr4rust.jar' - make sure this is the correct version with Rust support");
        "antlr4rust.jar".to_string()
    });

    // Check if the JAR file exists
    if !PathBuf::from(&antlr_jar).exists() {
        eprintln!("\n");
        eprintln!("ERROR: Could not find ANTLR4 JAR file at '{}'", antlr_jar);
        eprintln!(
            "\nTo generate ANTLR code, you must download the special ANTLR4 JAR with Rust support."
        );
        eprintln!("Download it from:");
        eprintln!("  https://github.com/rrevenantt/antlr4rust/releases/download/antlr4-4.8-2-Rust0.3.0-beta/antlr4-4.8-2-SNAPSHOT-complete.jar");
        eprintln!("\nThen either:");
        eprintln!("  1. Place it in build-tools/antlr4rust.jar");
        eprintln!("  2. Set ANTLR_JAR environment variable to the JAR path");
        eprintln!("\nRegular ANTLR4 JAR files DO NOT support generating Rust code!");
        eprintln!("\n");
        return Err("ANTLR4 JAR file not found - cannot generate parser code".into());
    }

    // Run the ANTLR tool to generate the parser code
    println!("cargo:warning=Generating ANTLR code from grammar files...");

    // Copy grammar files to a temporary directory for processing
    let temp_dir = env::temp_dir().join("substrait_antlr");
    fs::create_dir_all(&temp_dir)?;

    // Copy the grammar files
    let lexer_src = grammar_dir.join("SubstraitPlanLexer.g4");
    let parser_src = grammar_dir.join("SubstraitPlanParser.g4");
    let lexer_dst = temp_dir.join("SubstraitPlanLexer.g4");
    let parser_dst = temp_dir.join("SubstraitPlanParser.g4");

    fs::copy(&lexer_src, &lexer_dst)?;
    fs::copy(&parser_src, &parser_dst)?;

    println!(
        "cargo:warning=Copied grammar files to {}",
        temp_dir.display()
    );

    // First generate the lexer code
    println!("cargo:warning=Generating lexer code...");
    let lexer_cmd = format!(
        "java -jar {} -Dlanguage=Rust -visitor -o {} {}",
        antlr_jar,
        output_dir.display(),
        lexer_dst.display()
    );
    println!("cargo:warning=Executing: {}", lexer_cmd);

    let lexer_output = Command::new("java")
        .arg("-jar")
        .arg(&antlr_jar)
        .arg("-Dlanguage=Rust")
        .arg("-visitor")
        .arg("-o")
        .arg(&output_dir)
        .arg(&lexer_dst)
        .output()?;

    if !lexer_output.status.success() {
        println!(
            "cargo:warning=Lexer generation failed: {}",
            lexer_output.status
        );
        println!(
            "cargo:warning=STDOUT: {}",
            String::from_utf8_lossy(&lexer_output.stdout)
        );
        println!(
            "cargo:warning=STDERR: {}",
            String::from_utf8_lossy(&lexer_output.stderr)
        );
        return Err("ANTLR lexer generation failed".into());
    }

    println!("cargo:warning=Lexer generation succeeded");

    // Then generate the parser code
    println!("cargo:warning=Generating parser code...");
    let parser_cmd = format!(
        "java -jar {} -Dlanguage=Rust -visitor -o {} {}",
        antlr_jar,
        output_dir.display(),
        parser_dst.display()
    );
    println!("cargo:warning=Executing: {}", parser_cmd);

    let parser_output = Command::new("java")
        .arg("-jar")
        .arg(&antlr_jar)
        .arg("-Dlanguage=Rust")
        .arg("-visitor")
        .arg("-o")
        .arg(&output_dir)
        .arg(&parser_dst)
        .output()?;

    if !parser_output.status.success() {
        println!(
            "cargo:warning=Parser generation failed: {}",
            parser_output.status
        );
        println!(
            "cargo:warning=STDOUT: {}",
            String::from_utf8_lossy(&parser_output.stdout)
        );
        println!(
            "cargo:warning=STDERR: {}",
            String::from_utf8_lossy(&parser_output.stderr)
        );
        return Err("ANTLR parser generation failed".into());
    }

    println!("cargo:warning=Parser generation succeeded");
    println!("cargo:warning=ANTLR code generation completed successfully");

    // List the generated files
    println!("cargo:warning=Generated files:");
    if let Ok(entries) = fs::read_dir(&output_dir) {
        for entry in entries.flatten() {
            println!("cargo:warning=- {}", entry.path().display());
        }
    }

    // Fix known antlr4rust code generation bug: SubstraitPlanParserParserContext -> SubstraitPlanParserContext
    let parser_file = output_dir.join("substraitplanparser.rs");
    if parser_file.exists() {
        let content = fs::read_to_string(&parser_file)?;
        if content.contains("SubstraitPlanParserParserContext") {
            println!("cargo:warning=Fixing antlr4rust code generation bug in parser file");
            let fixed_content = content.replace(
                "SubstraitPlanParserParserContext",
                "SubstraitPlanParserContext",
            );
            fs::write(&parser_file, fixed_content)?;
            println!("cargo:warning=Parser bug fix applied successfully");
        }
    }

    // Format all generated ANTLR files with rustfmt
    println!("cargo:warning=Formatting generated ANTLR code");
    if let Ok(entries) = fs::read_dir(&output_dir) {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) == Some("rs") {
                let fmt_output = Command::new("rustfmt").arg(&path).output();
                match fmt_output {
                    Ok(output) if output.status.success() => {
                        println!("cargo:warning=Formatted {}", path.display());
                    }
                    Ok(output) => {
                        println!(
                            "cargo:warning=rustfmt failed for {}: {}",
                            path.display(),
                            output.status
                        );
                    }
                    Err(e) => {
                        println!(
                            "cargo:warning=Could not run rustfmt on {} (not installed?): {}",
                            path.display(),
                            e
                        );
                    }
                }
            }
        }
    }

    Ok(())
}

fn build_proto_descriptor(out_dir: &Path) -> Result<FileDescriptorSet, Box<dyn std::error::Error>> {
    let protos = WalkDir::new(PROTO_ROOT)
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
    cfg.compile_protos(&protos, &[PROTO_ROOT])?;

    let file_descriptor_contents = std::fs::read(&descriptor_path)?;
    let buf = bytes::Bytes::from(file_descriptor_contents);

    let file_descriptor_set = prost_types::FileDescriptorSet::decode(buf).unwrap();
    Ok(file_descriptor_set)
}

// Include the visitor generator module
mod visitor_generator {
    include!("build-tools/visitor_generator.rs");
}

/// Generate visitor code using our generator
fn generate_visitor_code() -> Result<(), Box<dyn std::error::Error>> {
    // Path to the output file
    let output_dir = PathBuf::from("src/textplan/converter/generated");
    let output_path = output_dir.join("base_plan_visitor.rs");

    // Create the output directory if it doesn't exist
    fs::create_dir_all(&output_dir)?;

    println!("cargo:warning=Generating visitor code from Substrait protobuf schema");
    let proto_descriptor = build_proto_descriptor(output_dir.as_path())?;

    // Run the visitor generator directly
    println!("cargo:warning=Output path: {}", output_path.display());
    visitor_generator::generate(proto_descriptor, &output_path)?;

    // Format the generated code with rustfmt
    println!("cargo:warning=Formatting generated visitor code");
    let fmt_output = Command::new("rustfmt").arg(&output_path).output();

    match fmt_output {
        Ok(output) if output.status.success() => {
            println!("cargo:warning=Generated code formatted successfully");
        }
        Ok(output) => {
            println!("cargo:warning=rustfmt failed: {}", output.status);
            println!(
                "cargo:warning=STDERR: {}",
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Err(e) => {
            println!(
                "cargo:warning=Could not run rustfmt (not installed?): {}",
                e
            );
        }
    }

    Ok(())
}
