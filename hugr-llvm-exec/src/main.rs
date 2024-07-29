use std::{error::Error, fs::File, path::{Path, PathBuf}, process::{Command, Stdio}};
use anyhow::{anyhow, Result};

use clap::Parser;
use hugr::{extension::{prelude, ExtensionRegistry}, std_extensions::arithmetic::{float_types, int_ops, int_types}, Hugr, HugrView};
use hugr_llvm::{custom::{tket2_qir::QirFunc, CodegenExtsMap}, emit::{EmitHugr, Namer}, fat::FatExt as _};
use inkwell::{context::Context, execution_engine, intrinsics::Intrinsic, module::{Linkage, Module}, targets::{Target, TargetMachine}, values::FunctionValue};
use tket2::extension::TKET2_EXTENSION;
use lazy_static::lazy_static;

lazy_static! {
    static ref EXTENSION_REGISTRY: ExtensionRegistry = ExtensionRegistry::try_new([
        int_ops::EXTENSION.to_owned(),
        int_types::EXTENSION.to_owned(),
        prelude::PRELUDE.to_owned(),
        float_types::EXTENSION.to_owned(),
        TKET2_EXTENSION.to_owned(),
    ])
    .unwrap();
}

#[derive(Parser)]
struct CliArgs {
    guppy_file: PathBuf
}

fn guppy(python_bin: impl AsRef<Path>, file: impl AsRef<Path>) -> Result<Hugr> {
    let mut guppy_cmd = Command::new(python_bin.as_ref());
    guppy_cmd
        .arg(file.as_ref())
        .stdout(Stdio::piped());
    let mut guppy_proc = guppy_cmd.spawn()?;
    let mut hugr: Hugr = serde_json::from_reader(guppy_proc.stdout.take().unwrap())?;
    if !guppy_proc.wait()?.success() {
        Err(anyhow!("Guppy failed"))?;
    }
    hugr.update_validate(&EXTENSION_REGISTRY).unwrap();
    Ok(hugr)
}

fn hugr_to_module<'c>(context: &'c Context, hugr: &'c impl HugrView) -> Result<(FunctionValue<'c>,Module<'c>)> {
    let module = context.create_module("hugr_llvm_exec");
    let namer = Namer::new("_hugr_llvm_exec_.", false);
    let exts = CodegenExtsMap::default()
        .add_int_extensions()
        .add_float_extensions()
        .add_tket2_qir_exts();
    let root = hugr.fat_root().unwrap();
    let module = EmitHugr::new(&context, module, namer.into(), exts.into())
        .emit_module(root)?
        .finish();
    let entry = {
        let guppy_entry = module.get_function("_hugr_llvm_exec_.main").ok_or(anyhow!("No main function"))?;
        let entry = module.add_function("main", context.void_type().fn_type(&[], false), Some(Linkage::External));
        let entry_block = context.append_basic_block(entry, "entry");
        let builder = context.create_builder();
        builder.position_at_end(entry_block);
        let debugtrap = Intrinsic::find("llvm.debugtrap").ok_or(anyhow!("Failed to find llvm.debugtrap"))?;
        let debugtrap = debugtrap.get_declaration(&module, &[]).ok_or(anyhow!("Failed to get declaration for llvm.debugtrap"))?;
        builder.build_call(debugtrap, &[], "")?;
        builder.build_call(guppy_entry, &[], "")?;
        builder.build_return(None)?;
        entry
    };
    Ok((entry, module))
}

fn jit_hugr(hugr: &impl HugrView) -> Result<()> {
    let context = inkwell::context::Context::create();
    let (entry, module) = hugr_to_module(&context, hugr)?;
    let engine = module.create_execution_engine().map_err(|e| anyhow!("Failed to create execution engine: {e}"))?;
    QirFunc::add_all_global_mappings(&engine, &module, |qir_func| {
        use hugr_llvm_exec::*;
        match qir_func {
            QirFunc::H => __quantum__qis__h__body as usize,
            QirFunc::RZ => __quantum__qis__rz__body as usize,
            QirFunc::QAlloc => __quantum__rt__qubit_allocate as usize,
            QirFunc::QFree => __quantum__rt__qubit_release as usize,
            QirFunc::Measure => __quantum__qis__m__body as usize,
            QirFunc::ReadResult => __quantum__qis__read_result__body as usize,
        }}
    )?;


    unsafe { engine.run_function(entry, &[])} ;
    Ok(())
}

// drives `hugr-llvm` to produce an LLVM module from a Hugr.
fn hugr_to_so<'c>(hugr: &'c Hugr) -> Result<()> {
    let context = inkwell::context::Context::create();
    let module = context.create_module("hugr_llvm_exec");
    let namer = Namer::new("_hugr_llvm_exec_.", false);
    let exts = CodegenExtsMap::default()
        .add_int_extensions()
        .add_float_extensions()
        .add_tket2_qir_exts();
    let root = hugr.fat_root().unwrap();
    let module = EmitHugr::new(&context, module, namer.into(), exts.into())
        .emit_module(root)?
        .finish();
    {
        let guppy_entry = module.get_function("_hugr_llvm_exec_.main").ok_or(anyhow!("No main function"))?;
        let entry = module.add_function("main", context.void_type().fn_type(&[], false), Some(Linkage::External));
        let entry_block = context.append_basic_block(entry, "entry");
        let builder = context.create_builder();
        builder.position_at_end(entry_block);
        builder.build_call(guppy_entry, &[], "")?;
        builder.build_return(None)?;
    }

    Target::initialize_native(&Default::default()).map_err(|e| anyhow!("Failed to initialize native target: {}", e))?;
    let triple = TargetMachine::get_default_triple();
    let target = Target::from_triple(&triple).map_err(|e| anyhow!("Failed to create target: {e}"))?;
    let cpu = TargetMachine::get_host_cpu_name().to_string();
    let cpu_features = TargetMachine::get_host_cpu_features().to_string();
    let machine = target.create_target_machine(&triple, &cpu, &cpu_features, inkwell::OptimizationLevel::None, inkwell::targets::RelocMode::PIC, inkwell::targets::CodeModel::Default).ok_or(anyhow!("Failed to create target machine"))?;

    machine.write_to_file(&module, inkwell::targets::FileType::Object, &PathBuf::from("hugr_llvm_exec.o")).map_err(|e| anyhow!("Failed to write object file: {}", e))?;

    if !Command::new("gcc")
        .arg("-Wl,-lhugr_llvm_exec")
        .arg("hugr_llvm_exec.o")
        .arg("-o")
        .arg("hugr_llvm_exec")
        .status()?.success() {
        Err(anyhow!("Failed to link object file"))?;
    }
    Ok(())
}


fn main_impl(args: CliArgs) -> Result<()> {
    let python = pathsearch::find_executable_in_path("python3").ok_or(anyhow!("Failed to find python3 executable"))?;
    if !args.guppy_file.exists() {
        Err(anyhow!("Guppy file does not exist: {:?}", args.guppy_file))?;
    }
    let hugr = guppy(python, args.guppy_file)?;
    jit_hugr(&hugr)
}

fn main() {
    if let Err(e) = main_impl(CliArgs::parse()) {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
