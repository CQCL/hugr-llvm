use std::{env, fs::{read_to_string, File}, path::{Path, PathBuf}, process::Command};

use insta::assert_snapshot;
use rstest::{fixture, rstest};
use insta_cmd::assert_cmd_snapshot;
use tempfile::{tempfile, NamedTempFile};

struct TestConfig {
    python_bin: PathBuf,
    hugr_llvm_bin: PathBuf,
    test_dir: PathBuf,
}

impl TestConfig {
    pub fn new() -> TestConfig {
        let python_bin = env::var("HUGR_LLVM_PYTHON_BIN")
            .map(Into::into)
            .ok()
            .or_else(|| pathsearch::find_executable_in_path("python"))
            .unwrap_or_else(|| panic!("Could not find python in PATH or HUGR_LLVM_PYTHON_BIN"));
        let hugr_llvm_bin = env!("CARGO_BIN_EXE_hugr-llvm").into();
        let test_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/guppy_test_cases").into();
        TestConfig {
            python_bin,
            hugr_llvm_bin,
            test_dir
        }
    }
}

impl TestConfig {
    fn get_guppy_output(&self, path: impl AsRef<Path>) -> NamedTempFile {
        let file = NamedTempFile::new().unwrap();
        let status = Command::new(&self.python_bin)
            .arg(self.test_dir.join(path.as_ref()))
            .arg("--mermaid")
            .stdout(file.reopen().unwrap())
            .status().unwrap();
        assert!(status.success(), "Failed to run guppy test case: {:?}", path.as_ref());
        file
    }

    fn hugr_llvm(&self, json_file: impl AsRef<Path>) -> Command {
        let mut command = Command::new(&self.hugr_llvm_bin);
        command.arg(json_file.as_ref());
        command
    }
}
#[fixture]
fn test_config() -> TestConfig {
    TestConfig::new()
}

#[rstest]
fn test_dir_exists(test_config: TestConfig) {
    assert!(test_config.test_dir.is_dir())
}

#[rstest]
fn test_even_odd(test_config: TestConfig) {
    let json_file = test_config.get_guppy_output("even_odd.py");
    let v: serde_json::Value = serde_json::from_reader(File::open(&json_file).unwrap()).unwrap();
    // println!("{}", serde_json::to_string_pretty(&v).unwrap());
    assert_cmd_snapshot!(test_config.hugr_llvm(&json_file))
}
