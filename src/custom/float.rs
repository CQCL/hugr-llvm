use anyhow::{anyhow, Result};
use hugr::{
    ops::{constant::CustomConst, CustomOp}, std_extensions::arithmetic::float_types::{self, ConstF64, FLOAT64_CUSTOM_TYPE, FLOAT64_TYPE}, types::CustomType, HugrView
};
use inkwell::{
    types::{BasicType, BasicTypeEnum, FloatType},
    values::{BasicValue, BasicValueEnum},
};

use crate::{emit::{func::EmitFuncContext, EmitOp, EmitOpArgs}, types::TypingSession};

use super::{CodegenExtensionsBuilder};

struct FloatOpsCodegenExtension;

fn float_type_handler<'c, H>(
    session: &TypingSession<'c, H>,
    hugr_type: &CustomType,
) -> Result<BasicTypeEnum<'c>> {
    debug_assert_eq!(hugr_type, &FLOAT64_CUSTOM_TYPE);
    Ok(session.iw_context().f64_type().as_basic_type_enum())
}

fn const_float_handler<'c, H: HugrView>(
    context: &mut EmitFuncContext<'c, H>,
    konst: &ConstF64,
) -> Result<BasicValueEnum<'c>> {
    let ty: FloatType<'c> = context.llvm_type(&konst.get_type())?.try_into().unwrap();
    Ok(ty.const_float(konst.value()).as_basic_value_enum())
}

// we allow dead code for now, but once we implement the emitter, we should
// remove this
#[allow(dead_code)]
struct FloatOpEmitter<'c, 'd, H>(&'d mut EmitFuncContext<'c, H>);

impl<'c, H: HugrView> EmitOp<'c, CustomOp, H> for FloatOpEmitter<'c, '_, H> {
    fn emit(&mut self, args: EmitOpArgs<'c, CustomOp, H>) -> Result<()> {
        use hugr::ops::NamedOp;
        let name = args.node().name();
        // This looks strange now, but we will add cases for ops piecemeal, as
        // in the analgous match expression in `IntOpEmitter`.
        #[allow(clippy::match_single_binding)]
        match name.as_str() {
            n => Err(anyhow!("FloatOpEmitter: unknown op: {n}")),
        }
    }
}

pub fn add_float_extensions<H: HugrView>(cem: CodegenExtensionsBuilder<H>) -> CodegenExtensionsBuilder<H> {
    cem.add_custom_const(const_float_handler).add_type(&FLOAT64_CUSTOM_TYPE, float_type_handler)
}

impl<H: HugrView> CodegenExtensionsBuilder<'_, H> {
    pub fn add_float_extensions(self) -> Self {
        add_float_extensions(self)
    }
}

#[cfg(test)]
mod test {
    use hugr::{
        builder::{Dataflow, DataflowSubContainer},
        std_extensions::arithmetic::{
            float_ops::FLOAT_OPS_REGISTRY,
            float_types::{ConstF64, FLOAT64_TYPE},
        },
    };
    use rstest::rstest;

    use super::add_float_extensions;
    use crate::{
        check_emission,
        emit::test::SimpleHugrConfig,
        test::{llvm_ctx, TestContext},
    };

    #[rstest]
    fn const_float(mut llvm_ctx: TestContext) {
        llvm_ctx.add_extensions(add_float_extensions);
        let hugr = SimpleHugrConfig::new()
            .with_outs(FLOAT64_TYPE)
            .with_extensions(FLOAT_OPS_REGISTRY.to_owned())
            .finish(|mut builder| {
                let c = builder.add_load_value(ConstF64::new(3.12));
                builder.finish_with_outputs([c]).unwrap()
            });
        check_emission!(hugr, llvm_ctx);
    }
}
