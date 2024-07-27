use hugr::{extension::simple_op::MakeExtensionOp, ops::{CustomOp, NamedOp, Value}, HugrView};
use inkwell::{builder::Builder, context::Context, types::BasicType, values::FunctionValue, AddressSpace};
use anyhow::{anyhow,Result};
use tket2::Tk2Op;

use crate::{emit::{emit_value, EmitFuncContext, EmitOp, EmitOpArgs}, types::TypingSession};

use super::{prelude::PreludeCodegen, CodegenExtension, CodegenExtsMap};

pub struct Tket2QIRPrelude;

const QUBIT_NAME: &str = "Qubit";
const RESULT_NAME: &str = "Result";
fn qubit_t(context: &Context) -> impl BasicType<'_> {
    context.get_struct_type(QUBIT_NAME).unwrap_or(context.opaque_struct_type(QUBIT_NAME))
        .ptr_type(AddressSpace::default())
}

fn result_t(context: &Context) -> impl BasicType<'_> {
    context.get_struct_type(RESULT_NAME).unwrap_or(context.opaque_struct_type(RESULT_NAME))
        .ptr_type(AddressSpace::default())
}

impl PreludeCodegen for Tket2QIRPrelude {
    fn qubit_type<'c,H: HugrView>(&self, session: &TypingSession<'c,H>) -> impl BasicType<'c> {
        qubit_t(session.iw_context())
    }
}

pub struct Tket2QIR;

impl<'c, H: HugrView> CodegenExtension<'c,H> for Tket2QIR {
    fn extension(&self) -> hugr::extension::ExtensionId {
        tket2::extension::TKET2_EXTENSION_ID
    }

    fn llvm_type(
        &self,
        _: &TypingSession<'c, H>,
        hugr_type: &hugr::types::CustomType,
    ) -> anyhow::Result<inkwell::types::BasicTypeEnum<'c>> {
        Err(anyhow::anyhow!(
            "Type not supported by tket2 qir extension: {hugr_type:?}"
        ))
    }

    fn emitter<'a>(
        &self,
        context: &'a mut EmitFuncContext<'c, H>,
    ) -> Box<dyn EmitOp<'c, CustomOp, H> + 'a> {
        Box::new(Tket2QIREmitter::new(context))
    }
}

pub struct Tket2QIREmitter<'c,'d,H: HugrView> (
     &'d mut EmitFuncContext<'c, H>,
);

impl<'c, 'd, H: HugrView> Tket2QIREmitter<'c, 'd, H> {
    fn new(context: &'d mut EmitFuncContext<'c,H>) -> Self { Self(context) }
    fn iw_context(&self) -> &'c Context {
        self.0.iw_context()
    }

    fn builder(&self) -> &Builder<'c> {
        self.0.builder()
    }

    fn get_func_h(&self) -> Result<FunctionValue<'c>> {
        self.0.get_extern_func(
            "__quantum__qis__h__body",
            self.iw_context().void_type().fn_type(
                &[qubit_t(self.iw_context()).as_basic_type_enum().into()],
                false,
            ),
        )
    }

    fn get_func_rz(&self) -> Result<FunctionValue<'c>> {
        self.0.get_extern_func(
            "__quantum__qis__rz__body",
            self.iw_context().void_type().fn_type(
                &[self.iw_context().f64_type().into(), qubit_t(self.iw_context()).as_basic_type_enum().into()],
                false,
            ),
        )
    }

    fn get_func_qalloc(&self) -> Result<FunctionValue<'c>> {
        self.0.get_extern_func(
            "__quantum__rt__qubit_allocate",
            qubit_t(self.iw_context()).fn_type(
                &[],
                false,
            ),
        )
    }

    fn get_func_qfree(&self) -> Result<FunctionValue<'c>> {
        self.0.get_extern_func(
            "__quantum__rt__qubit_release",
            self.iw_context().void_type().fn_type(
                &[qubit_t(self.iw_context()).as_basic_type_enum().into()],
                false,
            ),
        )
    }

    fn get_func_measure(&self) -> Result<FunctionValue<'c>> {
        self.0.get_extern_func(
            "__quantum__qis__m__body",
            result_t(self.iw_context()).fn_type(
                &[qubit_t(self.iw_context()).as_basic_type_enum().into()],
                false,
            ),
        )
    }

    fn get_func_read_result(&self) -> Result<FunctionValue<'c>> {
        self.0.get_extern_func(
            "__quantum__qis__read_result__body",
            self.iw_context().bool_type().fn_type(
                &[result_t(self.iw_context()).as_basic_type_enum().into()],
                false,
            ),
        )
    }
    // fn get_func_zz(&mut self) -> Result<inkwell::values::FunctionValue<'c>> {
    //     self.0.get_or_insert_function(
    //         "tket2_zz",
    //         self.0.iw_context().void_type().fn_type(
    //             &[
    //                 self.0.iw_context().i64_type().ptr_type(AddressSpace::Generic),
    //                 self.0.iw_context().i64_type().ptr_type(AddressSpace::Generic),
    //             ],
    //             false,
    //         ),
    //     )
    // }

    // fn get_func_rzz(&mut self) -> Result<inkwell::values::FunctionValue<'c>> {
    //     self.0.get_or_insert_function(
    //         "tket2_rzz",
    //         self.0.iw_context().void_type().fn_type(
    //             &[
    //                 self.0.iw_context().i64_type().ptr_type(AddressSpace::Generic),
    //                 self.0.iw_context().i64_type().ptr_type(AddressSpace::Generic),
    //                 self.0.iw_context().f64_type(),
    //             ],
    //             false,
    //         ),
    //     )
    // }

    // fn get_func_rxy(&mut self) -> Result<inkwell::values::FunctionValue<'c>> {
    //     self.0.get_or_insert_function(
    //         "tket2_rxy",
    //         self.0.iw_context().void_type().fn_type(
    //             &[
    //                 self.0.iw_context().i64_type().ptr_type(AddressSpace::Generic),
    //                 self.0.iw_context().f64_type(),
    //                 self.0.iw_context().f64_type(),
    //             ],
    //             false,
    //         ),
    //     )
    // }

    // fn get_func_qalloc(&mut self) -> Result<inkwell::values::FunctionValue<'c>> {
    //     self.0.get_or_insert_function(
    //         "tket2_qalloc",
    //         self.0.iw_context().i64_type().fn_type(&[], false),
    //     )
    // }


}

impl<'c, H: HugrView> EmitOp<'c, CustomOp, H> for Tket2QIREmitter<'c, '_, H> {
    /// Function to help lower the tket-2 extension.
    fn emit(&mut self, args: EmitOpArgs<'c, CustomOp, H>) -> Result<()> {
        match args
            .node()
            .as_extension_op()
            .ok_or(anyhow!("Tket2Emitter: Unknown op: {}", args.node().name()))
            .and_then(|x| Ok(MakeExtensionOp::from_extension_op(x)?))?
        {
            Tk2Op::H => {
                let [qb] = args
                    .inputs
                    .try_into()
                    .map_err(|_| anyhow!("H expects one inputs"))?;
                let func = self.get_func_h()?;
                self.builder()
                    .build_call(func, &[qb.into()],"")?;
                args.outputs.finish(self.builder(), [qb])
            }
            Tk2Op::RzF64 => {
                let [qb, angle] = args
                    .inputs
                    .try_into()
                    .map_err(|_| anyhow!("RzF64 expects two inputs"))?;
                let func = self.get_func_rz()?;
                self.builder()
                    .build_call(func, &[angle.into(), qb.into()],"")?;
                args.outputs.finish(self.builder(), [qb])
            }
            // Tk2Op::ZZMax => {
            //     let [qb1, qb2] = args
            //         .inputs
            //         .try_into()
            //         .map_err(|_| anyhow!("ZZMax expects two inputs"))?;
            //     let zz = self.get_func_zz()?;
            //     self.builder()
            //         .build_call(zz, &[qb1.into(), qb2.into()], "zz")?;
            //     args.outputs.finish(self.builder(), [qb1, qb2])
            // }
            // Tk2Op::ZZPhase => {
            //     let [qb1, qb2, angle] = args
            //         .inputs
            //         .try_into()
            //         .map_err(|_| anyhow!("ZZPhase expects three inputs"))?;
            //     let rzz = self.get_func_rzz()?;
            //     self.builder()
            //         .build_call(rzz, &[qb1.into(), qb2.into(), angle.into()], "rzz")?;
            //     args.outputs.finish(self.builder(), [qb1, qb2])
            // }
            // Tk2Op::PhasedX => {
            //     let [qb, f1, f2] = args
            //         .inputs
            //         .try_into()
            //         .map_err(|_| anyhow!("PhasedX expects three inputs"))?;
            //     let rxy = self.get_func_rxy()?;
            //     self.builder()
            //         .build_call(rxy, &[qb.into(), f1.into(), f2.into()], "rxy")?;
            //     args.outputs.finish(self.builder(), [qb])
            // }
            Tk2Op::QAlloc => {
                let [] = args
                    .inputs
                    .try_into()
                    .map_err(|_| anyhow!("QAlloc expects no inputs"))?;
                let qalloc = self.get_func_qalloc()?;
                let qb = self
                    .builder()
                    .build_call(qalloc, &[], "qalloc")?
                    .try_as_basic_value()
                    .unwrap_left();
                args.outputs.finish(self.builder(), [qb])
            }
            Tk2Op::QFree => {
                let [qb] = args
                    .inputs
                    .try_into()
                    .map_err(|_| anyhow!("QFree expects one inputs"))?;
                let qfree = self.get_func_qfree()?;
                self.builder().build_call(qfree, &[qb.into()], "qfree")?;
                args.outputs.finish(self.builder(), [])
            }
            Tk2Op::Measure => {
                let [qb] = args
                    .inputs
                    .try_into()
                    .map_err(|_| anyhow!("Measure expects one inputs"))?;
                let measure = self.get_func_measure()?;
                let result = self
                    .builder()
                    .build_call(measure, &[qb.into()],"")?
                    .try_as_basic_value()
                    .unwrap_left();
                let read_result = self.get_func_read_result()?;
                let result_i1 = self.builder().build_call(read_result, &[result.into()], "")?.try_as_basic_value().unwrap_left();
                let true_val = emit_value(&mut self.0, &Value::true_val())?;
                let false_val = emit_value(&mut self.0, &Value::false_val())?;
                let result = self
                    .builder()
                    .build_select(result_i1.into_int_value(), true_val, false_val, "measure")?;
                args.outputs.finish(self.builder(), [qb, result])
            }
            n => Err(anyhow!("Unknown op {:?}", n)),
        }
    }
}

impl<'c,H: HugrView> CodegenExtsMap<'c,H> {
    pub fn add_tket2_qir_exts(self) -> Self {
        self.add_prelude_extensions(Tket2QIRPrelude)
            .add_cge(Tket2QIR)
    }
}
