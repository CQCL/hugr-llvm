//! Provides a command line interface to tket2-hseries
use std::rc::Rc;

use clap::Parser;
use hugr::std_extensions::arithmetic::{
    conversions::EXTENSION as CONVERSIONS_EXTENSION, float_ops::EXTENSION as FLOAT_OPS_EXTENSION,
    float_types::EXTENSION as FLOAT_TYPES_EXTENSION, int_ops::EXTENSION as INT_OPS_EXTENSION,
    int_types::EXTENSION as INT_TYPES_EXTENSION,
};
use hugr::std_extensions::logic::EXTENSION as LOGICS_EXTENSION;
use hugr::Hugr;
use inkwell::module::Module;
use thiserror::Error;

use anyhow::Result;
use hugr::extension::{ExtensionRegistry, PRELUDE};
use lazy_static::lazy_static;

use crate::custom::CodegenExtsMap;
use crate::emit::{EmitHugr, Namer};
use crate::fat::FatExt as _;

lazy_static! {
    /// A registry suitable for passing to `run`. Use this unless you have a
    /// good reason not to do so.
    pub static ref REGISTRY: ExtensionRegistry = ExtensionRegistry::try_new([
        PRELUDE.to_owned(),
        INT_OPS_EXTENSION.to_owned(),
        INT_TYPES_EXTENSION.to_owned(),
        CONVERSIONS_EXTENSION.to_owned(),
        FLOAT_OPS_EXTENSION.to_owned(),
        FLOAT_TYPES_EXTENSION.to_owned(),
        LOGICS_EXTENSION.to_owned(),
    ])
    .unwrap();
}

/// Arguments for `run`.
#[derive(Parser, Debug)]
#[command(version, about)]
pub struct CmdLineArgs {
    #[command(flatten)]
    base: hugr_cli::CmdLineArgs,
    module_name: String,
    mangle_prefix: String,
    mangle_node_suffix: bool,
}

#[derive(Error, Debug)]
pub enum CliError {
    #[error(transparent)]
    HugrCliError(#[from] hugr_cli::CliError),
    #[error(transparent)]
    Other(#[from] anyhow::Error),
}

pub fn emit_module<'c>(
    context: &'c inkwell::context::Context,
    hugr: &'c Hugr,
    module_name: impl AsRef<str>,
    namer: Rc<Namer>,
    exts: Rc<CodegenExtsMap<'c, Hugr>>,
) -> Result<Module<'c>> {
    let module = context.create_module(module_name.as_ref());
    let emit = EmitHugr::new(context, module, namer, exts);
    Ok(emit.emit_module(hugr.fat_root().unwrap())?.finish())
}

impl CmdLineArgs {
    /// Run the ngrte preparation and validation workflow with the given
    /// registry.
    pub fn run(&self, registry: &ExtensionRegistry) -> Result<()> {
        let hugr = self.base.run(registry)?;

        let context = inkwell::context::Context::create();
        let module = emit_module(
            &context,
            &hugr,
            &self.module_name,
            self.namer(),
            self.codegenexts(),
        )?;
        module
            .verify()
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
        println!("{}", module.print_to_string());
        Ok(())
    }

    /// Test whether a `level` message should be output.
    pub fn verbosity(&self, level: hugr_cli::Level) -> bool {
        self.base.verbosity(level)
    }

    fn namer(&self) -> Rc<Namer> {
        Namer::new(self.mangle_prefix.clone(), self.mangle_node_suffix).into()
    }

    fn codegenexts<'c>(&self) -> Rc<CodegenExtsMap<'c, Hugr>> {
        CodegenExtsMap::new().add_int_extensions().into()
    }
}
