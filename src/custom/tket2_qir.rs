use hugr::{extension::simple_op::MakeExtensionOp, ops::{CustomOp, NamedOp, Value}, HugrView};
use inkwell::{builder::Builder, context::Context, execution_engine::ExecutionEngine, module::Module, types::{BasicType, FunctionType}, values::FunctionValue, AddressSpace};
use anyhow::{anyhow,Result};
use tket2::Tk2Op;
use strum_macros::EnumIter;
use strum::IntoEnumIterator;

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

#[derive(Copy,Clone,PartialEq, Eq, EnumIter)]
pub enum QirFunc {
    H,
    RZ,
    QAlloc,
    QFree,
    Measure,
    ReadResult,
}

impl QirFunc {
    pub fn symbol(self) -> &'static str {
        match self {
            QirFunc::H => "__quantum__qis__h__body",
            QirFunc::RZ => "__quantum__qis__rz__body",
            QirFunc::QAlloc => "__quantum__rt__qubit_allocate",
            QirFunc::QFree => "__quantum__rt__qubit_release",
            QirFunc::Measure => "__quantum__qis__m__body",
            QirFunc::ReadResult => "__quantum__qis__read_result__body",
        }
    }

    pub fn func_type<'c>(self, context: &'c Context) -> FunctionType<'c> {
        match self {
            QirFunc::H =>
            context.void_type().fn_type(
                &[context.f64_type().into(), qubit_t(context).as_basic_type_enum().into()],
                false,
            ),
            QirFunc::RZ => context.void_type().fn_type(
                &[context.f64_type().into(), qubit_t(context).as_basic_type_enum().into()],
                false,
            ),
            QirFunc::QAlloc => qubit_t(context).fn_type(
                &[],
                false,
            ),
            QirFunc::QFree => context.void_type().fn_type(
                &[qubit_t(context).as_basic_type_enum().into()],
                false,
            ),
            QirFunc::Measure => result_t(context).fn_type(
                &[qubit_t(context).as_basic_type_enum().into()],
                false,
            ),
            QirFunc::ReadResult => context.bool_type().fn_type(
                &[result_t(context).as_basic_type_enum().into()],
                false,
            ),
        }
    }

    pub fn add_global_mapping<'c>(self, engine: &ExecutionEngine<'c>, module: &Module<'c>, addr: usize) -> Result<()>{
        let Some(func) = module.get_function(self.symbol()) else {
            return Ok(());
        };

        engine.add_global_mapping(&func, addr);
        Ok(())
    }

    pub fn add_all_global_mappings<'c>(engine: &ExecutionEngine<'c>, module: &Module<'c>, get_addr: impl Fn(Self) -> usize) -> Result<()> {
        for x in Self::iter() {
            x.add_global_mapping(engine, module, get_addr(x))?;
        }
        Ok(())

    }

    fn get_func<'c,H: HugrView>(self, context: & EmitFuncContext<'c, H>) -> Result<FunctionValue<'c>> {
        context.get_extern_func(self.symbol(), self.func_type(context.iw_context()))
    }
}

impl<'c, 'd, H: HugrView> Tket2QIREmitter<'c, 'd, H> {
    fn new(context: &'d mut EmitFuncContext<'c,H>) -> Self { Self(context) }
    fn iw_context(&self) -> &'c Context {
        self.0.iw_context()
    }

    fn builder(&self) -> &Builder<'c> {
        self.0.builder()
    }

    fn get_func(&self, func:QirFunc) -> Result<FunctionValue<'c>> {
        func.get_func(self.0)
    }
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
                let func = self.get_func(QirFunc::H)?;
                self.builder()
                    .build_call(func, &[qb.into()],"")?;
                args.outputs.finish(self.builder(), [qb])
            }
            Tk2Op::RzF64 => {
                let [qb, angle] = args
                    .inputs
                    .try_into()
                    .map_err(|_| anyhow!("RzF64 expects two inputs"))?;
                let func = self.get_func(QirFunc::RZ)?;
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
                let qalloc = self.get_func(QirFunc::QAlloc)?;
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
                let qfree = self.get_func(QirFunc::QFree)?;
                self.builder().build_call(qfree, &[qb.into()], "qfree")?;
                args.outputs.finish(self.builder(), [])
            }
            Tk2Op::Measure => {
                let [qb] = args
                    .inputs
                    .try_into()
                    .map_err(|_| anyhow!("Measure expects one inputs"))?;
                let measure = self.get_func(QirFunc::Measure)?;
                let result = self
                    .builder()
                    .build_call(measure, &[qb.into()],"")?
                    .try_as_basic_value()
                    .unwrap_left();
                let read_result = self.get_func(QirFunc::ReadResult)?;
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
