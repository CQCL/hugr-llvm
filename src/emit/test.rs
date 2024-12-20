use std::iter;

use crate::custom::CodegenExtsBuilder;
use crate::extension::int::add_int_extensions;
use crate::types::HugrFuncType;
use crate::utils::fat::FatNode;
use anyhow::{anyhow, Result};
use hugr::builder::DataflowSubContainer;
use hugr::builder::{
    BuildHandle, Container, DFGWrapper, Dataflow, HugrBuilder, ModuleBuilder, SubContainer,
};
use hugr::extension::prelude::{ConstUsize, BOOL_T, USIZE_T};
use hugr::extension::{ExtensionRegistry, EMPTY_REG, PRELUDE_REGISTRY};
use hugr::ops::constant::CustomConst;
use hugr::ops::handle::FuncID;
use hugr::ops::{CallIndirect, Tag, Value};
use hugr::std_extensions::arithmetic::int_ops::{self, INT_OPS_REGISTRY};
use hugr::std_extensions::arithmetic::int_types::ConstInt;
use hugr::types::{Signature, Type, TypeRow};
use hugr::{type_row, Hugr, HugrView};
use inkwell::module::Module;
use inkwell::passes::PassManager;
use inkwell::values::GenericValue;
use itertools::Itertools;
use rstest::rstest;

use crate::test::*;

use super::EmitHugr;

#[allow(clippy::upper_case_acronyms)]
pub type DFGW<'a> = DFGWrapper<&'a mut Hugr, BuildHandle<FuncID<true>>>;

pub struct SimpleHugrConfig {
    ins: TypeRow,
    outs: TypeRow,
    extensions: ExtensionRegistry,
}

/// A wrapper for a module into which our tests will emit hugr.
pub struct Emission<'c> {
    module: Module<'c>,
}

impl<'c> Emission<'c> {
    /// Create an `Emission` from a HUGR.
    pub fn emit_hugr<'a: 'c, H: HugrView>(
        hugr: FatNode<'c, hugr::ops::Module, H>,
        eh: EmitHugr<'c, 'a, H>,
    ) -> Result<Self> where {
        let module = eh.emit_module(hugr)?.finish();
        Ok(Self { module })
    }

    /// Create an `Emission` from an LLVM Module.
    pub fn new(module: Module<'c>) -> Self {
        Self { module }
    }

    // Verify the inner Module.
    pub fn verify(&self) -> Result<()> {
        self.module
            .verify()
            .map_err(|err| anyhow!("Failed to verify module: {err}"))
    }

    /// Return the inner module.
    pub fn module(&self) -> &Module<'c> {
        &self.module
    }

    /// Run passes on the inner module.
    pub fn opt(&self, go: impl FnOnce() -> PassManager<Module<'c>>) {
        go().run_on(&self.module);
    }

    // Print the inner module to stderr.
    pub fn print_module(&self) {
        self.module.print_to_stderr();
    }

    /// JIT and execute the function named `entry` in the inner module.
    ///
    /// That function must take no arguments and return an `i64`.
    pub fn exec_u64(&self, entry: impl AsRef<str>) -> Result<u64> {
        let gv = self.exec_impl(entry)?;
        Ok(gv.as_int(false))
    }

    /// JIT and execute the function named `entry` in the inner module.
    ///
    /// That function must take no arguments and return an `f64`.
    pub fn exec_f64(&self, entry: impl AsRef<str>) -> Result<f64> {
        let gv = self.exec_impl(entry)?;
        Ok(gv.as_float(&self.module.get_context().f64_type()))
    }

    pub(crate) fn exec_impl(&self, entry: impl AsRef<str>) -> Result<GenericValue<'c>> {
        let entry_fv = self
            .module
            .get_function(entry.as_ref())
            .ok_or_else(|| anyhow!("Function {} not found in module", entry.as_ref()))?;

        entry_fv.set_linkage(inkwell::module::Linkage::External);

        let ee = self
            .module
            .create_jit_execution_engine(inkwell::OptimizationLevel::None)
            .map_err(|err| anyhow!("Failed to create execution engine: {err}"))?;
        let fv = ee.get_function_value(entry.as_ref())?;
        Ok(unsafe { ee.run_function(fv, &[]) })
    }
}

impl SimpleHugrConfig {
    pub fn new() -> Self {
        Self {
            ins: Default::default(),
            outs: Default::default(),
            extensions: EMPTY_REG,
        }
    }

    pub fn with_ins(mut self, ins: impl Into<TypeRow>) -> Self {
        self.ins = ins.into();
        self
    }

    pub fn with_outs(mut self, outs: impl Into<TypeRow>) -> Self {
        self.outs = outs.into();
        self
    }

    pub fn with_extensions(mut self, extensions: ExtensionRegistry) -> Self {
        self.extensions = extensions;
        self
    }

    pub fn finish(
        self,
        make: impl for<'a> FnOnce(DFGW<'a>) -> <DFGW<'a> as SubContainer>::ContainerHandle,
    ) -> Hugr {
        self.finish_with_exts(|builder, _| make(builder))
    }
    pub fn finish_with_exts(
        self,
        make: impl for<'a> FnOnce(
            DFGW<'a>,
            &ExtensionRegistry,
        ) -> <DFGW<'a> as SubContainer>::ContainerHandle,
    ) -> Hugr {
        let mut mod_b = ModuleBuilder::new();
        let func_b = mod_b
            .define_function("main", HugrFuncType::new(self.ins, self.outs))
            .unwrap();
        make(func_b, &self.extensions);

        // Intentionally left as a debugging aid. If the HUGR you construct
        // fails validation, uncomment the following line to print it out
        // unvalidated.
        // println!("{}", mod_b.hugr().mermaid_string());

        mod_b.finish_hugr(&self.extensions).unwrap()
    }
}

impl Default for SimpleHugrConfig {
    fn default() -> Self {
        Self::new()
    }
}

/// A macro used to check the emission of a Hugr module,
/// and to assert the correctness of the emitted LLVM IR using [insta].
///
/// Call with
/// ```ignore
/// check_emission!(hugr, llvm_ctx);
/// ```
/// or
/// ```ignore
/// check_emission!("snapshot_name", hugr, llvm_ctx);
/// ```
#[macro_export]
macro_rules! check_emission {
    // Call the macro with a snapshot name.
    ($snapshot_name:expr, $hugr: ident, $test_ctx:ident) => {{
        let root = $crate::utils::fat::FatExt::fat_root::<hugr::ops::Module>(&$hugr).unwrap();
        let emission =
            $crate::emit::test::Emission::emit_hugr(root, $test_ctx.get_emit_hugr()).unwrap();

        let mut settings = insta::Settings::clone_current();
        let new_suffix = settings
            .snapshot_suffix()
            .map_or("pre-mem2reg".into(), |x| format!("pre-mem2reg@{x}"));
        settings.set_snapshot_suffix(new_suffix);
        settings.bind(|| {
            let mod_str = emission.module().to_string();
            if $snapshot_name == "" {
                insta::assert_snapshot!(mod_str)
            } else {
                insta::assert_snapshot!($snapshot_name, mod_str)
            }
        });

        emission.verify().unwrap();

        emission.opt(|| {
            let pb = inkwell::passes::PassManager::create(());
            pb.add_promote_memory_to_register_pass();
            pb
        });

        let mod_str = emission.module().to_string();
        if $snapshot_name == "" {
            insta::assert_snapshot!(mod_str)
        } else {
            insta::assert_snapshot!($snapshot_name, mod_str)
        }
        emission
    }};
    // Use the default snapshot name.
    ($hugr: ident, $test_ctx:ident) => {
        check_emission!("", $hugr, $test_ctx)
    };
}

#[rstest]
fn emit_hugr_tag(llvm_ctx: TestContext) {
    let hugr = SimpleHugrConfig::new()
        .with_outs(Type::new_unit_sum(3))
        .finish(|mut builder: DFGW| {
            let tag = builder
                .add_dataflow_op(
                    Tag::new(1, vec![vec![].into(), vec![].into(), vec![].into()]),
                    builder.input_wires(),
                )
                .unwrap();
            builder.finish_with_outputs(tag.outputs()).unwrap()
        });
    let _ = check_emission!(hugr, llvm_ctx);
}

#[rstest]
fn emit_hugr_dfg(llvm_ctx: TestContext) {
    let hugr = SimpleHugrConfig::new()
        .with_ins(Type::UNIT)
        .with_outs(Type::UNIT)
        .finish(|mut builder: DFGW| {
            let dfg = {
                let b = builder
                    .dfg_builder(HugrFuncType::new_endo(Type::UNIT), builder.input_wires())
                    .unwrap();
                let w = b.input_wires();
                b.finish_with_outputs(w).unwrap()
            };
            builder.finish_with_outputs(dfg.outputs()).unwrap()
        });
    check_emission!(hugr, llvm_ctx);
}

#[rstest]
fn emit_hugr_conditional(llvm_ctx: TestContext) {
    let hugr = {
        let input_v_rows: Vec<TypeRow> = (0..3).map(Type::new_unit_sum).map_into().collect_vec();
        let output_v_rows = {
            let mut r = input_v_rows.clone();
            r.reverse();
            r
        };

        SimpleHugrConfig::new()
            .with_ins(vec![Type::new_sum(input_v_rows.clone()), Type::UNIT])
            .with_outs(vec![Type::new_sum(output_v_rows.clone()), Type::UNIT])
            .finish(|mut builder: DFGW| {
                let cond = {
                    let [sum_input, other_input] = builder.input_wires_arr();
                    let mut cond_b = builder
                        .conditional_builder(
                            (input_v_rows.clone(), sum_input),
                            [(Type::UNIT, other_input)],
                            vec![Type::new_sum(output_v_rows.clone()), Type::UNIT].into(),
                        )
                        .unwrap();
                    for i in 0..3 {
                        let mut case_b = cond_b.case_builder(i).unwrap();
                        let [case_input, other_input] = case_b.input_wires_arr();
                        let tag = case_b
                            .add_dataflow_op(Tag::new(2 - i, output_v_rows.clone()), [case_input])
                            .unwrap();
                        case_b
                            .finish_with_outputs([tag.out_wire(0), other_input])
                            .unwrap();
                    }
                    cond_b.finish_sub_container().unwrap()
                };
                let [o1, o2] = cond.outputs_arr();
                builder.finish_with_outputs([o1, o2]).unwrap()
            })
    };
    check_emission!(hugr, llvm_ctx);
}

#[rstest]
fn emit_hugr_load_constant(mut llvm_ctx: TestContext) {
    llvm_ctx.add_extensions(add_int_extensions);
    let v = Value::tuple([
        Value::unit_sum(2, 4).unwrap(),
        ConstInt::new_s(4, -24).unwrap().into(),
    ]);

    let hugr = SimpleHugrConfig::new()
        .with_outs(v.get_type())
        .with_extensions(INT_OPS_REGISTRY.to_owned())
        .finish(|mut builder: DFGW| {
            let konst = builder.add_load_value(v);
            builder.finish_with_outputs([konst]).unwrap()
        });
    check_emission!(hugr, llvm_ctx);
}

#[rstest]
fn emit_hugr_call(llvm_ctx: TestContext) {
    fn build_recursive(mod_b: &mut ModuleBuilder<Hugr>, name: &str, io: TypeRow) {
        let f_id = mod_b
            .declare(name, HugrFuncType::new_endo(io).into())
            .unwrap();
        let mut func_b = mod_b.define_declaration(&f_id).unwrap();
        let call = func_b
            .call(&f_id, &[], func_b.input_wires(), &EMPTY_REG)
            .unwrap();
        func_b.finish_with_outputs(call.outputs()).unwrap();
    }

    let mut mod_b = ModuleBuilder::new();
    build_recursive(&mut mod_b, "main_void", type_row![]);
    build_recursive(&mut mod_b, "main_unary", type_row![BOOL_T]);
    build_recursive(&mut mod_b, "main_binary", type_row![BOOL_T, BOOL_T]);
    let hugr = mod_b.finish_hugr(&EMPTY_REG).unwrap();
    check_emission!(hugr, llvm_ctx);
}

#[rstest]
fn emit_hugr_call_indirect(llvm_ctx: TestContext) {
    fn build_recursive(mod_b: &mut ModuleBuilder<Hugr>, name: &str, io: TypeRow) {
        let signature = HugrFuncType::new_endo(io);
        let f_id = mod_b.declare(name, signature.clone().into()).unwrap();
        let mut func_b = mod_b.define_declaration(&f_id).unwrap();
        let func = func_b.load_func(&f_id, &[], &EMPTY_REG).unwrap();
        let inputs = iter::once(func).chain(func_b.input_wires());
        let call_indirect = func_b
            .add_dataflow_op(CallIndirect { signature }, inputs)
            .unwrap();
        func_b.finish_with_outputs(call_indirect.outputs()).unwrap();
    }

    let mut mod_b = ModuleBuilder::new();
    build_recursive(&mut mod_b, "main_void", type_row![]);
    build_recursive(&mut mod_b, "main_unary", type_row![BOOL_T]);
    build_recursive(&mut mod_b, "main_binary", type_row![BOOL_T, BOOL_T]);
    let hugr = mod_b.finish_hugr(&EMPTY_REG).unwrap();
    check_emission!(hugr, llvm_ctx);
}

#[rstest]
fn emit_hugr_custom_op(mut llvm_ctx: TestContext) {
    llvm_ctx.add_extensions(add_int_extensions);
    let v1 = ConstInt::new_s(4, -24).unwrap();
    let v2 = ConstInt::new_s(4, 24).unwrap();

    let hugr = SimpleHugrConfig::new()
        .with_outs(v1.get_type())
        .with_extensions(INT_OPS_REGISTRY.to_owned())
        .finish_with_exts(|mut builder: DFGW, ext_reg| {
            let k1 = builder.add_load_value(v1);
            let k2 = builder.add_load_value(v2);
            let ext_op = int_ops::EXTENSION
                .instantiate_extension_op("iadd", [4.into()], ext_reg)
                .unwrap();
            let add = builder.add_dataflow_op(ext_op, [k1, k2]).unwrap();
            builder.finish_with_outputs(add.outputs()).unwrap()
        });
    check_emission!(hugr, llvm_ctx);
}

#[rstest]
fn get_external_func(llvm_ctx: TestContext) {
    let emc = llvm_ctx.get_emit_module_context();
    let func_type1 = emc.iw_context().i32_type().fn_type(&[], false);
    let func_type2 = emc.iw_context().f64_type().fn_type(&[], false);
    let foo1 = emc.get_extern_func("foo", func_type1).unwrap();
    assert_eq!(foo1.get_name().to_str().unwrap(), "foo");
    let foo2 = emc.get_extern_func("foo", func_type1).unwrap();
    assert_eq!(foo1, foo2);
    assert!(emc.get_extern_func("foo", func_type2).is_err());
}

#[rstest]
fn diverse_module_children(llvm_ctx: TestContext) {
    let hugr = {
        let mut builder = ModuleBuilder::new();
        let _ = {
            let fbuilder = builder
                .define_function("f1", HugrFuncType::new_endo(type_row![]))
                .unwrap();
            fbuilder.finish_sub_container().unwrap()
        };
        let _ = {
            let fbuilder = builder
                .define_function("f2", HugrFuncType::new_endo(type_row![]))
                .unwrap();
            fbuilder.finish_sub_container().unwrap()
        };
        let _ = builder.add_constant(Value::true_val());
        let _ = builder
            .declare("decl", HugrFuncType::new_endo(type_row![]).into())
            .unwrap();
        builder.finish_hugr(&EMPTY_REG).unwrap()
    };
    check_emission!(hugr, llvm_ctx);
}

#[rstest]
fn diverse_dfg_children(llvm_ctx: TestContext) {
    let hugr = SimpleHugrConfig::new()
        .with_outs(BOOL_T)
        .finish(|mut builder: DFGW| {
            let [r] = {
                let mut builder = builder
                    .dfg_builder(HugrFuncType::new(type_row![], BOOL_T), [])
                    .unwrap();
                let konst = builder.add_constant(Value::false_val());
                let func = {
                    let mut builder = builder
                        .define_function("scoped_func", HugrFuncType::new(type_row![], BOOL_T))
                        .unwrap();
                    let w = builder.load_const(&konst);
                    builder.finish_with_outputs([w]).unwrap()
                };
                let [r] = builder
                    .call(func.handle(), &[], [], &EMPTY_REG)
                    .unwrap()
                    .outputs_arr();
                builder.finish_with_outputs([r]).unwrap().outputs_arr()
            };
            builder.finish_with_outputs([r]).unwrap()
        });
    check_emission!(hugr, llvm_ctx);
}

#[rstest]
fn diverse_cfg_children(llvm_ctx: TestContext) {
    let hugr = SimpleHugrConfig::new()
        .with_outs(BOOL_T)
        .finish(|mut builder: DFGW| {
            let [r] = {
                let mut builder = builder.cfg_builder([], type_row![BOOL_T]).unwrap();
                let konst = builder.add_constant(Value::false_val());
                let func = {
                    let mut builder = builder
                        .define_function("scoped_func", HugrFuncType::new(type_row![], BOOL_T))
                        .unwrap();
                    let w = builder.load_const(&konst);
                    builder.finish_with_outputs([w]).unwrap()
                };
                let entry = {
                    let mut builder = builder
                        .entry_builder([type_row![]], type_row![BOOL_T])
                        .unwrap();
                    let control = builder.add_load_value(Value::unary_unit_sum());
                    let [r] = builder
                        .call(func.handle(), &[], [], &EMPTY_REG)
                        .unwrap()
                        .outputs_arr();
                    builder.finish_with_outputs(control, [r]).unwrap()
                };
                let exit = builder.exit_block();
                builder.branch(&entry, 0, &exit).unwrap();
                builder.finish_sub_container().unwrap().outputs_arr()
            };
            builder.finish_with_outputs([r]).unwrap()
        });
    check_emission!(hugr, llvm_ctx);
}

#[rstest]
fn load_function(llvm_ctx: TestContext) {
    let hugr = {
        let mut builder = ModuleBuilder::new();
        let target_sig = Signature::new_endo(type_row![]);
        let target_func = builder
            .declare("target_func", target_sig.clone().into())
            .unwrap();
        let _ = {
            let mut builder = builder
                .define_function(
                    "main",
                    Signature::new(type_row![], Type::new_function(target_sig)),
                )
                .unwrap();
            let r = builder.load_func(&target_func, &[], &EMPTY_REG).unwrap();
            builder.finish_with_outputs([r]).unwrap()
        };
        builder.finish_hugr(&EMPTY_REG).unwrap()
    };

    check_emission!(hugr, llvm_ctx);
}

#[rstest]
fn test_exec(mut exec_ctx: TestContext) {
    let hugr = SimpleHugrConfig::new()
        .with_outs(USIZE_T)
        .with_extensions(PRELUDE_REGISTRY.to_owned())
        .finish(|mut builder: DFGW| {
            let konst = builder.add_load_value(ConstUsize::new(42));
            builder.finish_with_outputs([konst]).unwrap()
        });
    exec_ctx.add_extensions(CodegenExtsBuilder::add_default_prelude_extensions);
    assert_eq!(42, exec_ctx.exec_hugr_u64(hugr, "main"));
}
