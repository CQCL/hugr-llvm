use std::io::Write as _;
use std::rc::Rc;

use clap::Parser;
use clap_verbosity_flag::Level;
use hugr::{Hugr, HugrView};
use hugr_cli::HugrArgs;

use anyhow::{anyhow, Result};
use inkwell::context::Context;
use itertools::Itertools;

use crate::fat::FatExt as _;
use crate::{
    custom::CodegenExtsMap,
    emit::{EmitHugr, Namer},
};

#[derive(Parser, Debug)]
pub struct CliArgs {
    #[clap(flatten)]
    pub hugr_args: HugrArgs,

    pub module_name: Option<String>,

    /// Output file '-' for stdout
    #[clap(long, short, value_parser, default_value = "-")]
    pub output: clio::Output,
}

impl CliArgs {
    pub fn verbosity(&self, level: Level) -> bool {
        self.hugr_args.verbosity(level)
    }

    pub fn run(mut self, extensions: Rc<CodegenExtsMap<'static, Hugr>>) -> Result<()> {
        let (hugrs, _registry) = self.hugr_args.validate()?;

        let hugr = hugrs
            .into_iter()
            .exactly_one()
            .map_err(|_| anyhow!("Exactly one Hugr must be specified"))?;

        let context = Context::create();
        let namer = Namer::default();
        let llvm_module =
            context.create_module(&self.module_name.unwrap_or_else(|| "hugr-llvm".to_string()));

        let hugr_module = hugr
            .as_ref()
            .fat_root()
            .ok_or(anyhow!("Hugr root must be a module"))?;

        let llvm_module = EmitHugr::new(&context, llvm_module, namer.into(), extensions)
            .emit_module(hugr_module)?
            .finish();

        write!(self.output, "{}", llvm_module.to_string())?;

        Ok(())
    }
}
