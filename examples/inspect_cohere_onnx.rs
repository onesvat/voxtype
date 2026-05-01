//! Quick inspector for the cstr/cohere-transcribe-onnx-int8 model files.
//! Run with:
//!   cargo run --example inspect_cohere_onnx --features cohere -- \
//!     ~/.cache/voxtype-models/cohere-transcribe-int8

use std::env;
use std::path::PathBuf;

use ort::session::Session;
use ort::value::ValueType;

fn type_str(ty: &ValueType) -> String {
    match ty {
        ValueType::Tensor {
            ty,
            shape,
            dimension_symbols,
        } => {
            let dims: Vec<String> = shape
                .iter()
                .zip(dimension_symbols.iter())
                .map(|(d, sym)| {
                    if *d < 0 {
                        let s: &str = sym.as_ref();
                        if s.is_empty() {
                            "?".into()
                        } else {
                            format!("?:{s}")
                        }
                    } else {
                        d.to_string()
                    }
                })
                .collect();
            format!("Tensor<{ty:?}>[{}]", dims.join(", "))
        }
        other => format!("{other:?}"),
    }
}

fn dump(label: &str, path: &PathBuf) -> ort::Result<()> {
    println!("\n=== {label} ===");
    println!("file: {}", path.display());
    let session = Session::builder()?.commit_from_file(path)?;
    println!("inputs:");
    for (i, input) in session.inputs().iter().enumerate() {
        println!("  [{i}] {} : {}", input.name(), type_str(input.dtype()));
    }
    println!("outputs:");
    for (i, output) in session.outputs().iter().enumerate() {
        println!("  [{i}] {} : {}", output.name(), type_str(output.dtype()));
    }
    Ok(())
}

fn main() -> ort::Result<()> {
    let dir: PathBuf = env::args()
        .nth(1)
        .map(PathBuf::from)
        .expect("usage: inspect_cohere_onnx <model-dir>");
    // Tolerate one model failing to load so we get whichever is ready.
    if let Err(e) = dump("encoder", &dir.join("cohere-encoder.int8.onnx")) {
        println!("encoder failed: {e}");
    }
    if let Err(e) = dump("decoder", &dir.join("cohere-decoder.int8.onnx")) {
        println!("decoder failed: {e}");
    }
    Ok(())
}
