use std::{process::exit, rc::Rc};

use clap::Parser;
use hugr::Hugr;
use hugr_llvm::{
    cli::CliArgs,
    custom::{CodegenExtsBuilder, CodegenExtsMap},
};

fn extensions() -> Rc<CodegenExtsMap<'static, Hugr>> {
    CodegenExtsBuilder::default()
        .add_default_prelude_extensions()
        .add_float_extensions()
        .add_rotation_extensions()
        .add_tket2_quantum_qir_extensions()
        .add_tket2_results_extensions()
        .finish()
        .into()
}

fn main() {
    let args = CliArgs::parse();
    let report_errors = args.verbosity(clap_verbosity_flag::Level::Error);
    if let Err(e) = args.run(extensions()) {
        if report_errors {
            eprintln!("{e}")
        }
        exit(1);
    }
}
