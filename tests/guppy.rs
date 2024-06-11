use std::{env, path::PathBuf};

use rstest::{fixture, rstest};

struct TestConfig {
    python_bin: PathBuf,
    hugr_llvm_bin: PathBuf,
}

impl TestConfig {
    pub fn new() -> TestConfig {
        let python_bin = env::var("HUGR_LLVM_PYTHON_BIN")
            .map(Into::into)
            .ok()
            .or_else(|| pathsearch::find_executable_in_path("python"))
            .unwrap_or_else(|| panic!("Could not find python in PATH or HUGR_LLVM_PYTHON_BIN"));
        let hugr_llvm_bin = env!("CARGO_BIN_EXE_hugr-llvm").into();
        TestConfig {
            python_bin,
            hugr_llvm_bin,
        }
    }
}

#[fixture]
fn test_config() -> TestConfig {
    TestConfig::new()
}

#[rstest]
fn test_even_odd(test_config: TestConfig) {}
