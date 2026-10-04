use std::{env, fs, path::Path};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var_os("CARGO_FEATURE_EMBEDDED_RUNTIME").is_none() {
        return;
    }
    println!("cargo:rerun-if-changed=embedded/runtime-stubs.bin");

    let manifest_dir = env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    let artifact_path = Path::new(&manifest_dir).join("embedded/runtime-stubs.bin");
    let artifact = fs::read(&artifact_path).unwrap_or_else(|error| {
        panic!(
            "failed to read embedded runtime stub artifact {}: {error}",
            artifact_path.display()
        )
    });
    if artifact.is_empty() {
        panic!(
            "embedded runtime stub artifact is empty: {}",
            artifact_path.display()
        );
    }

    let generated = format!(
        "pub(crate) const EMBEDDED_STUB_ARTIFACT: &[u8] = include_bytes!({:?});\n",
        artifact_path.to_string_lossy()
    );
    let output_path =
        Path::new(&env::var("OUT_DIR").expect("OUT_DIR not set")).join("embedded_stub_artifact.rs");
    if fs::read_to_string(&output_path).ok().as_deref() != Some(generated.as_str()) {
        fs::write(&output_path, generated).expect("failed to write embedded stub artifact");
    }
}
