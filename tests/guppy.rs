use std::{
    env,
    fmt::Display,
    path::{Path, PathBuf},
    process::{Command, Stdio},
};

use insta_cmd::assert_cmd_snapshot;
use rstest::{fixture, rstest};

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
    fn get_guppy_output(&self, path: impl AsRef<Path>) -> Command {
        let mut cmd = Command::new(&self.python_bin);
        cmd.arg(self.test_dir.join(path.as_ref()));
        cmd
    }

    fn hugr_llvm(&self) -> Command {
        let mut cmd = Command::new(&self.hugr_llvm_bin);
        if !self.opt {
            cmd.arg("--no-opt");
        }
        cmd
    }

    fn run<T>(&self, path: impl AsRef<Path>, go: impl FnOnce(Command) -> T) -> T {
        let mut guppy_cmd = self.get_guppy_output(path);
        guppy_cmd.stdout(Stdio::piped());
        let mut guppy_proc = guppy_cmd.spawn().expect("Failed to start guppy");

        let mut hugr_llvm = self.hugr_llvm();
        hugr_llvm.stdin(guppy_proc.stdout.take().unwrap()).arg("-");
        let r = go(hugr_llvm);
        assert!(guppy_proc.wait().unwrap().success());
        r
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
            with_suffix("noopt", || {
                test_config.opt = false;
                test_config.run($filename, |mut cmd| assert_cmd_snapshot!(cmd));
            });
            with_suffix("opt", || {
                test_config.opt = true;
                test_config.run($filename, |mut cmd| assert_cmd_snapshot!(cmd));
            });
        }
    };
}

guppy_test!("even_odd.py", even_odd);
guppy_test!("even_odd2.py", even_odd2);
guppy_test!("planqc-1.py", planqc1);
guppy_test!("planqc-2.py", planqc2);
guppy_test!("planqc-3.py", planqc3);
