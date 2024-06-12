use std::{
    env,
    fmt::Display,
    path::{Path, PathBuf},
    process::Command,
};

use insta_cmd::assert_cmd_snapshot;
use rstest::{fixture, rstest};
use tempfile::NamedTempFile;

struct TestConfig {
    python_bin: PathBuf,
    hugr_llvm_bin: PathBuf,
    test_dir: PathBuf,
    pub opt: bool,
}

impl TestConfig {
    pub fn new() -> TestConfig {
        let python_bin = env::var("HUGR_LLVM_PYTHON_BIN")
            .map(Into::into)
            .ok()
            .or_else(|| pathsearch::find_executable_in_path("python"))
            .unwrap_or_else(|| panic!("Could not find python in PATH or HUGR_LLVM_PYTHON_BIN"));
        let hugr_llvm_bin = env!("CARGO_BIN_EXE_hugr-llvm").into();
        let test_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/guppy_test_cases")
            .into();
        TestConfig {
            python_bin,
            hugr_llvm_bin,
            test_dir,
            opt: true,
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
            .status()
            .unwrap();
        assert!(
            status.success(),
            "Failed to run guppy test case: {:?}",
            path.as_ref()
        );
        file
    }

    fn hugr_llvm(&self, json_file: impl AsRef<Path>) -> Command {
        let mut command = Command::new(&self.hugr_llvm_bin);
        command.arg(json_file.as_ref());
        if !self.opt {
            command.arg("--no-opt");
        }
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

fn with_suffix<R>(s: impl Display, go: impl FnOnce() -> R) -> R {
    let mut settings = insta::Settings::clone_current();
    let old_suffix = settings
        .snapshot_suffix()
        .map_or("".to_string(), |s| format!("{s}."));
    let llvm_str = hugr_llvm::llvm_version();
    settings.set_snapshot_suffix(format!("{old_suffix}{llvm_str}.{s}"));
    settings.bind(go)
}

macro_rules! guppy_test {
    ($filename:expr, $testname:ident) => {
        #[rstest]
        fn $testname(mut test_config: TestConfig) {
            let json_file = test_config.get_guppy_output($filename);
            with_suffix("noopt", || {
                test_config.opt = false;
                assert_cmd_snapshot!(test_config.hugr_llvm(&json_file))
            });
            with_suffix("opt", || {
                test_config.opt = true;
                assert_cmd_snapshot!(test_config.hugr_llvm(&json_file))
            });
        }
    };
}

guppy_test!("even_odd.py", even_odd);
