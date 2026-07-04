# Build Tools

This directory contains tools used by the build system:

- `antlr4-4.8-2-SNAPSHOT-complete.jar`: Special version of ANTLR4 with Rust support
- `visitor_generator.rs`: Protocol buffer visitor generator module used by build.rs

## Visitor Generator

The visitor generator (`visitor_generator.rs`) is used directly by the build.rs script to generate a
BasePlanProtoVisitor trait and implementation for the Substrait protocol buffers. This automatically runs whenever the
project is built.

The generator follows a two-phase approach:

1. Discovery phase: Parse protobuf definitions to build a type inventory
2. Generation phase: Generate visitor code based on the discovered types

The output is written to `src/textplan/converter/generated/base_plan_visitor.rs`.