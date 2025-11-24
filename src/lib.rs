// SPDX-License-Identifier: Apache-2.0

//! Substrait textplan is a library for parsing and generating Substrait plans in both
//! text and binary formats. This Rust implementation mirrors the C++ implementation
//! and is designed to be usable from other languages through FFI.

pub mod proto;
pub mod textplan;

use std::ffi::{c_char, CStr, CString};
use std::ptr;

// Re-export key types
pub use textplan::common::error::TextPlanError;
pub use textplan::common::parse_result::ParseResult;
pub use textplan::printer::plan_printer::TextPlanFormat;
pub use textplan::symbol_table::SymbolTable;

/// Parses a textplan string and returns a ParseResult.
///
/// # Arguments
///
/// * `text` - The textplan string to parse.
///
/// # Returns
///
/// A ParseResult containing the resulting SymbolTable and any errors.
pub fn parse_text_plan(text: &str) -> ParseResult {
    textplan::parser::parse_text::parse_stream(text)
}

/// Loads a binary Substrait plan and converts it to a textplan string.
///
/// # Arguments
///
/// * `bytes` - The binary plan to load.
///
/// # Returns
///
/// The textplan representation of the plan, or an error.
pub fn binary_to_text_plan(bytes: &[u8]) -> Result<String, TextPlanError> {
    textplan::converter::save_text::save_to_text(bytes)
}

/// Serializes a symbol table to a textplan string.
///
/// # Arguments
///
/// * `symbol_table` - The symbol table to serialize.
/// * `format` - The format to use for the output.
///
/// # Returns
///
/// The textplan string representation of the symbol table, or an error.
pub fn symbol_table_to_text_plan(
    symbol_table: &SymbolTable,
    format: TextPlanFormat,
) -> Result<String, TextPlanError> {
    textplan::parser::parse_text::serialize_to_text(symbol_table, format)
}

/// FFI API for loading a textplan from a string and converting it to binary protobuf
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn load_from_text(text_ptr: *const c_char) -> *mut u8 {
    if text_ptr.is_null() {
        return ptr::null_mut();
    }

    let c_str = unsafe { CStr::from_ptr(text_ptr) };
    let text = match c_str.to_str() {
        Ok(s) => s,
        Err(_) => return ptr::null_mut(),
    };

    match textplan::parser::load_from_text(text) {
        Ok(plan_bytes) => {
            // Allocate memory for the binary plan that will be returned to C/C++
            let len = plan_bytes.len();
            let result_size = len + std::mem::size_of::<usize>();

            let layout =
                std::alloc::Layout::from_size_align(result_size, 8).expect("Invalid layout");

            unsafe {
                let ptr = std::alloc::alloc(layout);
                if ptr.is_null() {
                    return ptr::null_mut();
                }

                // First write the length
                let len_ptr = ptr as *mut usize;
                *len_ptr = len;

                // Then write the actual data
                let data_ptr = ptr.add(std::mem::size_of::<usize>());
                std::ptr::copy_nonoverlapping(plan_bytes.as_ptr(), data_ptr, len);

                ptr
            }
        }
        Err(_) => ptr::null_mut(),
    }
}

/// FFI API for freeing memory allocated by this library
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn free_plan_bytes(ptr: *mut u8) {
    if ptr.is_null() {
        return;
    }

    unsafe {
        let len_ptr = ptr as *const usize;
        let len = *len_ptr;
        let result_size = len + std::mem::size_of::<usize>();

        let layout = std::alloc::Layout::from_size_align(result_size, 8).expect("Invalid layout");

        std::alloc::dealloc(ptr, layout);
    }
}

/// FFI API for saving a binary plan to textplan format
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn save_to_text(bytes_ptr: *const u8, bytes_len: usize) -> *mut c_char {
    if bytes_ptr.is_null() {
        return ptr::null_mut();
    }

    let bytes = unsafe { std::slice::from_raw_parts(bytes_ptr, bytes_len) };

    match textplan::converter::save_to_text(bytes) {
        Ok(text_plan) => match CString::new(text_plan) {
            Ok(c_string) => c_string.into_raw(),
            Err(_) => ptr::null_mut(),
        },
        Err(_) => ptr::null_mut(),
    }
}

/// FFI API for freeing memory allocated by this library
#[no_mangle]
#[allow(clippy::not_unsafe_ptr_arg_deref)]
pub extern "C" fn free_text_plan(text_ptr: *mut c_char) {
    if !text_ptr.is_null() {
        unsafe {
            let _ = CString::from_raw(text_ptr);
            // CString destructor will free the memory
        }
    }
}
