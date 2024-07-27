use std::{error::Error, fs::File, path::{Path, PathBuf}, process::{Command, Stdio}};
use anyhow::{anyhow, Result};

use clap::Parser;
use findshlibs::{Segment, SharedLibrary, TargetSharedLibrary};
use hugr::{extension::ExtensionRegistry, Hugr, extension::prelude,
           std_extensions::arithmetic::{float_types, int_ops, int_types}};
use hugr_llvm::{custom::CodegenExtsMap, emit::{EmitHugr, Namer}, fat::FatExt as _};
use inkwell::{module::{Linkage, Module}, targets::{Target, TargetMachine}};
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

fn run(lib: impl AsRef<Path>, entry: impl AsRef<str>) -> Result<()> {
    unsafe {
        let runtime = libloading::Library::new("libhugr_llvm_exec.so")?;
        let lib = libloading::Library::new(&format!("./{}", lib.as_ref().to_string_lossy()))?;

        let func: libloading::Symbol<unsafe extern "C" fn()> = lib.get(entry.as_ref().as_bytes())?;
        TargetSharedLibrary::each(|shlib| {
            println!("{}", shlib.name().to_string_lossy());

            for seg in shlib.segments() {
                println!("    {}: segment {}",
                        seg.actual_virtual_memory_address(shlib),
                        seg.name().to_string());
            }
        });
        Ok(func())
    }
}

fn main_impl(args: CliArgs) -> Result<()> {
    let python = pathsearch::find_executable_in_path("python3").ok_or(anyhow!("Failed to find python3 executable"))?;
    if !args.guppy_file.exists() {
        Err(anyhow!("Guppy file does not exist: {:?}", args.guppy_file))?;
    }
    let hugr = guppy(python, args.guppy_file)?;
    hugr_to_so(&hugr)?;
    run("hugr_llvm_exec.so", "_hugr_llvm_exec_entry")?;
    Ok(())
}

fn main() {
    if let Err(e) = main_impl(CliArgs::parse()) {
        eprintln!("Error: {}", e);
        std::process::exit(1);
    }
}
