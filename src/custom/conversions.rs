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
    types::{CustomType, TypeArg},
    HugrView,
};

use inkwell::{
    intrinsics::Intrinsic,
    types::{BasicType, BasicTypeEnum},
    values::{AnyValue, BasicValue},
    FloatPredicate,
};

use crate::{
    emit::{
        func::EmitFuncContext,
        ops::{emit_custom_unary_op, emit_value},
        EmitOp, EmitOpArgs,
    },
    types::TypingSession,
};

struct ConversionsEmitter<'c, 'd, H>(&'d mut EmitFuncContext<'c, H>);

impl<'c, H: HugrView> EmitOp<'c, ExtensionOp, H> for ConversionsEmitter<'c, '_, H> {
    fn emit(&mut self, args: EmitOpArgs<'c, ExtensionOp, H>) -> Result<()> {
        let conversion_op = ConvertOpDef::from_optype(&args.node().generalise()).ok_or(anyhow!(
            "ConversionsEmitter from_optype failed: {:?}",
            args.node().as_ref()
        ))?;

        match conversion_op {
            ConvertOpDef::trunc_u | ConvertOpDef::trunc_s => {
                let signed = conversion_op == ConvertOpDef::trunc_s;

                // This op should have one type arg only: the log-width of the
                // int we're truncating to.
                let Some(TypeArg::BoundedNat { n: log_width }) =
                    conversion_op.type_args().last().cloned()
                else {
                    panic!("Unexpected type args to truncate node")
                };

                // Note: This logic is copied from `llvm_type` in the IntTypes
                // extension. We need to have a common source of truth for this.
                let width = match log_width {
                    0..=3 => Ok(8),
                    4 => Ok(16),
                    5 => Ok(32),
                    6 => Ok(64),
                    m => Err(anyhow!(
                        "IntTypesCodegenExtension: unsupported log_width: {}",
                        m
                    )),
                }?;

                let hugr_int_ty = INT_TYPES[log_width as usize].clone();
                let int_ty = self
                    .0
                    .typing_session()
                    .llvm_type(&hugr_int_ty)?
                    .into_int_type();

                let hugr_sum_ty = sum_with_error(vec![hugr_int_ty]);
                let sum_ty = self.0.typing_session().llvm_sum_type(hugr_sum_ty)?;

                emit_custom_unary_op(self.0, args, |ctx, arg, _| {
                    // We have to check if the conversion will work, so we
                    // make the maximum int and convert to a float, then compare
                    // with the function input.
                    let int_max = int_ty.const_all_ones();

                    let flt_int_max = if signed {
                        let abs_name = &format!("llvm.abs.i{}", width);
                        let abs_intr = Intrinsic::find(&abs_name)
                            .expect(&format!("Couldn't find {} intrinsic", abs_name));
                        let abs = abs_intr
                            .get_declaration(
                                ctx.get_current_module(),
                                &[int_ty.as_basic_type_enum()],
                            )
                            .unwrap();

                        let abs_call = ctx
                            .builder()
                            .build_call(abs, &[arg.into()], "max_int")?
                            .as_any_value_enum()
                            .into_int_value();

                        ctx.builder().build_signed_int_to_float(
                            abs_call,
                            ctx.iw_context().f64_type(),
                            "max_flt",
                        )
                    } else {
                        ctx.builder().build_unsigned_int_to_float(
                            int_max,
                            ctx.iw_context().f64_type(),
                            "max_flt",
                        )
                    }?;

                    // Build fabs intrinsic
                    let fabs_intr =
                        Intrinsic::find("llvm.fabs.f64").expect("Couldn't find fabs intrinsic");
                    let fabs = fabs_intr
                        .get_declaration(
                            ctx.get_current_module(),
                            &[ctx.iw_context().f64_type().as_basic_type_enum()],
                        )
                        .ok_or(anyhow!("TODO"))?;

                    let fabs_call = ctx
                        .builder()
                        .build_call(fabs, &[arg.into()], "flt_pos")?
                        .as_any_value_enum()
                        .into_float_value();

                    // TODO: Test this with converting INT_MAX to float to see if we need a cheeky error margin
                    let success = ctx.builder().build_float_compare(
                        FloatPredicate::OLE,
                        fabs_call,
                        flt_int_max,
                        "conversion_valid",
                    )?;

                    // Perform the conversion unconditionally, which will result
                    // in a poison value if the input was too large. We will
                    // decide whether we return it based on the result of our
                    // earlier check.
                    let trunc_result = if signed {
                        ctx.builder()
                            .build_float_to_signed_int(arg.into_float_value(), int_ty, "")
                    } else {
                        ctx.builder().build_float_to_unsigned_int(
                            arg.into_float_value(),
                            int_ty,
                            "",
                        )
                    }?
                    .as_basic_value_enum();

                    let trunc_err_hugr_val = Value::extension(ConstError::new(
                        2,
                        format!(
                            "Float value too big to convert to int of given width ({})",
                            width
                        ),
                    ));
                    let e = emit_value(ctx, &trunc_err_hugr_val)?;

                    // Make a struct with both fields (error message and
                    // conversion result) populated, then set the tag to the
                    // to the result of our overflow check.
                    // This should look the same as the appropriate sum instance.
                    let val = sum_ty
                        .get_poison()
                        .as_basic_value_enum()
                        .into_struct_value();
                    let val = ctx.builder().build_insert_value(val, e, 1, "error val")?;
                    let val = ctx.builder().build_insert_value(
                        val,
                        trunc_result,
                        2,
                        "conversion_result",
                    )?;
                    let val = ctx.builder().build_insert_value(val, success, 0, "tag")?;

                    Ok(vec![val.as_basic_value_enum()])
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
