use super::{CodegenExtension, CodegenExtsMap};

use anyhow::{anyhow, Result};

use hugr::{
    extension::{
        prelude::{sum_with_error, ConstError},
        simple_op::MakeExtensionOp,
        ExtensionId,
    },
    ops::{constant::Value, custom::ExtensionOp},
    std_extensions::arithmetic::{
        conversions::{self, ConvertOpDef},
        int_types::INT_TYPES,
    },
    types::CustomType,
    HugrView,
};

use inkwell::types::BasicTypeEnum;
use inkwell::values::{AnyValue, BasicValue};

use crate::{
    emit::{
        func::EmitFuncContext,
        ops::{emit_custom_unary_op, emit_value},
        EmitOp, EmitOpArgs,
    },
    types::TypingSession,
};

struct ConversionsEmitter<'c, 'd, H>(&'d mut EmitFuncContext<'c, H>);

fn log2(m: u32) -> Option<u8> {
    if m == 1 {
        Some(0)
    } else if m % 2 == 0 {
        let n = log2(m / 2)?;
        Some(n + 1)
    } else {
        None
    }
}

impl<'c, H: HugrView> EmitOp<'c, ExtensionOp, H> for ConversionsEmitter<'c, '_, H> {
    fn emit(&mut self, args: EmitOpArgs<'c, ExtensionOp, H>) -> Result<()> {
        let conversion_op = ConvertOpDef::from_optype(&args.node().generalise()).ok_or(anyhow!(
            "ConversionsEmitter from_optype failed: {:?}",
            args.node().as_ref()
        ))?;

        match conversion_op {
            ConvertOpDef::trunc_u | ConvertOpDef::trunc_s => {
                let out_sum_ty = args
                    .outputs
                    .get_types()
                    .last()
                    .map(|a| a.into_struct_type())
                    .unwrap();
                let int_ty = out_sum_ty
                    .get_field_type_at_index(2)
                    .unwrap()
                    .into_struct_type()
                    .get_field_type_at_index(0)
                    .unwrap()
                    .into_int_type();
                let width = int_ty.get_bit_width();
                let log_width = log2(width).unwrap();
                let hugr_sum_ty = sum_with_error(vec![INT_TYPES[log_width as usize].clone()]);
                let sum_ty = self.0.typing_session().llvm_sum_type(hugr_sum_ty)?;
                emit_custom_unary_op(self.0, args, |ctx, arg, _| {
                    let trunc_result = if conversion_op == ConvertOpDef::trunc_u {
                        ctx.builder().build_float_to_unsigned_int(
                            arg.into_float_value(),
                            int_ty,
                            "",
                        )
                    } else {
                        // Otherwise it's ConvertOpDef::trunc_s
                        ctx.builder()
                            .build_float_to_signed_int(arg.into_float_value(), int_ty, "")
                    }?
                    .as_basic_value_enum();
                    let optional_result = if trunc_result.is_poison() {
                        let trunc_err_hugr_val = Value::extension(ConstError::new(
                            2,
                            format!(
                                "Float value too big to convert to int of given width ({})",
                                width
                            ),
                        ));
                        let e = emit_value(ctx, &trunc_err_hugr_val)?;
                        sum_ty.build_tag(ctx.builder(), 0, vec![e])
                    } else {
                        sum_ty.build_tag(ctx.builder(), 1, vec![trunc_result])
                    }?;

                    Ok(vec![optional_result])
                })
            }

            ConvertOpDef::convert_u => emit_custom_unary_op(self.0, args, |ctx, arg, out_tys| {
                let out_ty = out_tys.last().unwrap();
                Ok(vec![ctx
                    .builder()
                    .build_unsigned_int_to_float(
                        arg.into_int_value(),
                        out_ty.into_float_type(),
                        "",
                    )?
                    .as_basic_value_enum()])
            }),

            ConvertOpDef::convert_s => emit_custom_unary_op(self.0, args, |ctx, arg, out_tys| {
                let out_ty = out_tys.last().unwrap();
                Ok(vec![ctx
                    .builder()
                    .build_signed_int_to_float(arg.into_int_value(), out_ty.into_float_type(), "")?
                    .as_basic_value_enum()])
            }),
            _ => Err(anyhow!(
                "Conversion op not implemented: {:?}",
                args.node().as_ref()
            )),
        }
        //Ok(())
    }
}

pub struct ConversionsCodegenExtension;

impl<'c, H: HugrView> CodegenExtension<'c, H> for ConversionsCodegenExtension {
    fn extension(&self) -> ExtensionId {
        conversions::EXTENSION_ID
    }

    fn llvm_type<'d>(
        &self,
        _context: &TypingSession<'c, H>,
        hugr_type: &CustomType,
    ) -> Result<BasicTypeEnum<'c>> {
        Err(anyhow!(
            "IntOpsCodegenExtension: unsupported type: {}",
            hugr_type
        ))
    }

    fn emitter<'a>(
        &self,
        context: &'a mut EmitFuncContext<'c, H>,
    ) -> Box<dyn EmitOp<'c, ExtensionOp, H> + 'a> {
        Box::new(ConversionsEmitter(context))
    }
}

pub fn add_conversions_extension<H: HugrView>(cem: CodegenExtsMap<'_, H>) -> CodegenExtsMap<'_, H> {
    cem.add_cge(ConversionsCodegenExtension)
}

#[cfg(test)]
mod test {

    use super::*;

    use crate::check_emission;
    use crate::custom::{
        float::add_float_extensions, int::add_int_extensions,
        prelude::add_default_prelude_extensions,
    };
    use crate::emit::test::SimpleHugrConfig;
    use crate::test::{llvm_ctx, TestContext};
    use hugr::{
        builder::{Dataflow, DataflowSubContainer},
        std_extensions::arithmetic::{
            conversions::{CONVERT_OPS_REGISTRY, EXTENSION},
            float_types::FLOAT64_TYPE,
            int_types::INT_TYPES,
        },
        types::Type,
        Hugr,
    };
    use rstest::rstest;

    fn test_conversion_op(
        name: impl AsRef<str>,
        in_type: Type,
        out_type: Type,
        int_width: u8,
    ) -> Hugr {
        SimpleHugrConfig::new()
            .with_ins(vec![in_type.clone()])
            .with_outs(vec![out_type.clone()])
            .with_extensions(CONVERT_OPS_REGISTRY.clone())
            .finish(|mut hugr_builder| {
                let [in1] = hugr_builder.input_wires_arr();
                let ext_op = EXTENSION
                    .instantiate_extension_op(
                        name.as_ref(),
                        [(int_width as u64).into()],
                        &CONVERT_OPS_REGISTRY,
                    )
                    .unwrap();
                let outputs = hugr_builder
                    .add_dataflow_op(ext_op, [in1])
                    .unwrap()
                    .outputs();
                hugr_builder.finish_with_outputs(outputs).unwrap()
            })
    }

    #[rstest]
    #[case(256, Some(8))]
    #[case(63, None)]
    #[case(1, Some(0))]
    fn log(#[case] m: u32, #[case] expected: Option<u8>) {
        assert_eq!(log2(m), expected)
    }

    #[rstest]
    #[case("convert_u", 4)]
    #[case("convert_s", 5)]
    fn test_convert(mut llvm_ctx: TestContext, #[case] op_name: &str, #[case] width: u8) -> () {
        llvm_ctx.add_extensions(add_int_extensions);
        llvm_ctx.add_extensions(add_float_extensions);
        llvm_ctx.add_extensions(add_conversions_extension);
        let in_ty = INT_TYPES[width as usize].clone();
        let out_ty = FLOAT64_TYPE;
        let hugr = test_conversion_op(op_name, in_ty, out_ty, width);
        check_emission!(op_name, hugr, llvm_ctx);
    }

    #[rstest]
    #[case("trunc_u", 6)]
    #[case("trunc_s", 5)]
    fn test_truncation(mut llvm_ctx: TestContext, #[case] op_name: &str, #[case] width: u8) -> () {
        llvm_ctx.add_extensions(add_int_extensions);
        llvm_ctx.add_extensions(add_float_extensions);
        llvm_ctx.add_extensions(add_conversions_extension);
        llvm_ctx.add_extensions(add_default_prelude_extensions);
        let in_ty = FLOAT64_TYPE;
        let out_ty = sum_with_error(INT_TYPES[width as usize].clone());
        let hugr = test_conversion_op(op_name, in_ty, out_ty.into(), width);
        check_emission!(op_name, hugr, llvm_ctx);
    }
}
