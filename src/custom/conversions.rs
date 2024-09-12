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
        conversions::{self, ConvertOpDef, ConvertOpType},
        int_types::INT_TYPES,
    },
    types::{CustomType, TypeArg},
    HugrView,
};

use inkwell::{types::BasicTypeEnum, values::BasicValue, FloatPredicate};

use crate::{
    emit::{
        func::EmitFuncContext,
        ops::{emit_custom_unary_op, emit_value},
        EmitOp, EmitOpArgs,
    },
    types::TypingSession,
};

struct ConversionsEmitter<'c, 'd, H>(&'d mut EmitFuncContext<'c, H>);

impl<'c, H: HugrView> ConversionsEmitter<'c, '_, H> {
    fn build_trunc_op(
        &mut self,
        signed: bool,
        log_width: u64,
        args: EmitOpArgs<'c, ExtensionOp, H>,
    ) -> Result<()> {
        // Note: This logic is copied from `llvm_type` in the IntTypes
        // extension. We need to have a common source of truth for this.
        let (width, (int_min_value_s, int_max_value_s), int_max_value_u) = match log_width {
            0..=3 => (8, (i8::MIN as i64, i8::MAX as i64), u8::MAX as u64),
            4 => (16, (i16::MIN as i64, i16::MAX as i64), u16::MAX as u64),
            5 => (32, (i32::MIN as i64, i32::MAX as i64), u32::MAX as u64),
            6 => (64, (i64::MIN, i64::MAX), u64::MAX),
            m => {
                return Err(anyhow!(
                    "ConversionEmitter: unsupported log_width: {}",
                    m
                ))
            }
        };

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
            let flt_max = if signed {
                ctx.iw_context()
                    .f64_type()
                    .const_float(int_max_value_s as f64)
            } else {
                ctx.iw_context()
                    .f64_type()
                    .const_float(int_max_value_u as f64)
            };

            let within_upper_bound = ctx.builder().build_float_compare(
                FloatPredicate::OLE,
                arg.into_float_value(),
                flt_max,
                "within_upper_bound",
            )?;

            let flt_min = if signed {
                ctx.iw_context()
                    .f64_type()
                    .const_float(int_min_value_s as f64)
            } else {
                ctx.iw_context().f64_type().const_float(0.0)
            };

            let within_lower_bound = ctx.builder().build_float_compare(
                FloatPredicate::OLE,
                flt_min,
                arg.into_float_value(),
                "within_lower_bound",
            )?;

            // N.B. If the float value is NaN, we will never succeed.
            let success = ctx
                .builder()
                .build_and(within_upper_bound, within_lower_bound, "success")
                .unwrap();

            // Perform the conversion unconditionally, which will result
            // in a poison value if the input was too large. We will
            // decide whether we return it based on the result of our
            // earlier check.
            let trunc_result = if signed {
                ctx.builder().build_float_to_signed_int(
                    arg.into_float_value(),
                    int_ty,
                    "trunc_result",
                )
            } else {
                ctx.builder().build_float_to_unsigned_int(
                    arg.into_float_value(),
                    int_ty,
                    "trunc_result",
                )
            }?
            .as_basic_value_enum();

            let err_msg = Value::extension(ConstError::new(
                2,
                format!(
                    "Float value too big to convert to int of given width ({})",
                    width
                ),
            ));

            let err_val = emit_value(ctx, &err_msg)?;
            let failure = sum_ty.build_tag(ctx.builder(), 0, vec![err_val]).unwrap();
            let trunc_result = sum_ty
                .build_tag(ctx.builder(), 1, vec![trunc_result])
                .unwrap();

            let final_result = ctx
                .builder()
                .build_select(success, trunc_result, failure, "")
                .unwrap();
            Ok(vec![final_result])
        })
    }
}

impl<'c, H: HugrView> EmitOp<'c, ExtensionOp, H> for ConversionsEmitter<'c, '_, H> {
    fn emit(&mut self, args: EmitOpArgs<'c, ExtensionOp, H>) -> Result<()> {
        let conversion_op =
            ConvertOpType::from_optype(&args.node().generalise()).ok_or(anyhow!(
                "ConversionsEmitter from_optype failed: {:?}",
                args.node().as_ref()
            ))?;

        match conversion_op.def() {
            ConvertOpDef::trunc_u | ConvertOpDef::trunc_s => {
                let signed = conversion_op.def() == &ConvertOpDef::trunc_s;
                let Some(TypeArg::BoundedNat { n: log_width }) =
                    conversion_op.type_args().last().cloned()
                else {
                    panic!("This op should have one type arg only: the log-width of the int we're truncating to.")
                };

                self.build_trunc_op(signed, log_width, args)
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
            // These ops convert between hugr's `USIZE` and u64. The former is
            // implementation-dependent and we define them to be the same.
            // Hence our implementation is a noop.
            ConvertOpDef::itousize | ConvertOpDef::ifromusize => {
                emit_custom_unary_op(self.0, args, |_, arg, _| Ok(vec![arg]))
            }
            _ => Err(anyhow!(
                "Conversion op not implemented: {:?}",
                args.node().as_ref()
            )),
        }
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
            "ConversionEmitter: unsupported type: {}",
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
    use crate::emit::test::{SimpleHugrConfig, DFGW};
    use crate::test::{exec_ctx, llvm_ctx, TestContext};
    use hugr::builder::SubContainer;
    use hugr::{
        builder::{Dataflow, DataflowSubContainer},
        extension::prelude::{ConstUsize, PRELUDE_REGISTRY, USIZE_T},
        std_extensions::arithmetic::{
            conversions::{ConvertOpDef, CONVERT_OPS_REGISTRY, EXTENSION},
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
    fn test_convert(mut llvm_ctx: TestContext, #[case] op_name: &str, #[case] log_width: u8) -> () {
        llvm_ctx.add_extensions(add_int_extensions);
        llvm_ctx.add_extensions(add_float_extensions);
        llvm_ctx.add_extensions(add_conversions_extension);
        let in_ty = INT_TYPES[log_width as usize].clone();
        let out_ty = FLOAT64_TYPE;
        let hugr = test_conversion_op(op_name, in_ty, out_ty, log_width);
        check_emission!(op_name, hugr, llvm_ctx);
    }

    #[rstest]
    #[case("trunc_u", 6)]
    #[case("trunc_s", 5)]
    fn test_truncation(mut llvm_ctx: TestContext, #[case] op_name: &str, #[case] log_width: u8) -> () {
        llvm_ctx.add_extensions(add_int_extensions);
        llvm_ctx.add_extensions(add_float_extensions);
        llvm_ctx.add_extensions(add_conversions_extension);
        llvm_ctx.add_extensions(add_default_prelude_extensions);
        let in_ty = FLOAT64_TYPE;
        let out_ty = sum_with_error(INT_TYPES[log_width as usize].clone());
        let hugr = test_conversion_op(op_name, in_ty, out_ty.into(), log_width);
        check_emission!(op_name, hugr, llvm_ctx);
    }

    #[rstest]
    fn my_test_exec(mut exec_ctx: TestContext) {
        let hugr = SimpleHugrConfig::new()
            .with_outs(USIZE_T)
            .with_extensions(PRELUDE_REGISTRY.to_owned())
            .finish(|mut builder: DFGW| {
                let konst = builder.add_load_value(ConstUsize::new(42));
                builder.finish_with_outputs([konst]).unwrap()
            });
        exec_ctx.add_extensions(add_default_prelude_extensions);
        assert_eq!(42, exec_ctx.exec_hugr_u64(hugr, "main"));
    }

    #[rstest]
    #[case(0)]
    #[case(42)]
    #[case(18_446_744_073_709_551_615)]
    fn usize_roundtrip(mut exec_ctx: TestContext, #[case] val: u64) -> () {
        let hugr = SimpleHugrConfig::new()
            .with_outs(USIZE_T)
            .with_extensions(CONVERT_OPS_REGISTRY.clone())
            .finish(|mut builder: DFGW| {
                let k = builder.add_load_value(ConstUsize::new(val));
                let [int] = builder
                    .add_dataflow_op(ConvertOpDef::ifromusize.without_log_width(), [k])
                    .unwrap()
                    .outputs_arr();
                let [usize_] = builder
                    .add_dataflow_op(ConvertOpDef::itousize.without_log_width(), [int])
                    .unwrap()
                    .outputs_arr();
                builder.finish_with_outputs([usize_]).unwrap()
            });
        exec_ctx.add_extensions(add_int_extensions);
        exec_ctx.add_extensions(add_conversions_extension);
        exec_ctx.add_extensions(add_default_prelude_extensions);
        assert_eq!(val, exec_ctx.exec_hugr_u64(hugr, "main"));
    }

    fn roundtrip_hugr(val: u64) -> Hugr {
        let int64 = INT_TYPES[6].clone();
        SimpleHugrConfig::new()
            .with_outs(USIZE_T)
            .with_extensions(CONVERT_OPS_REGISTRY.clone())
            .finish(|mut builder| {
                let k = builder.add_load_value(ConstUsize::new(val));
                let [int] = builder
                    .add_dataflow_op(ConvertOpDef::ifromusize.without_log_width(), [k])
                    .unwrap()
                    .outputs_arr();
                let [flt] = builder
                    .add_dataflow_op(ConvertOpDef::convert_u.with_log_width(6), [int])
                    .unwrap()
                    .outputs_arr();
                let [int_or_err] = builder
                    .add_dataflow_op(ConvertOpDef::trunc_u.with_log_width(6), [flt])
                    .unwrap()
                    .outputs_arr();
                let sum_ty = sum_with_error(int64.clone());
                let variants = (0..sum_ty.num_variants())
                    .map(|i| sum_ty.get_variant(i).unwrap().clone().try_into().unwrap());
                let mut cond_b = builder
                    .conditional_builder((variants, int_or_err), [], vec![int64].into())
                    .unwrap();
                let win_case = cond_b.case_builder(1).unwrap();
                let [win_in] = win_case.input_wires_arr();
                win_case.finish_with_outputs([win_in]).unwrap();
                let mut lose_case = cond_b.case_builder(0).unwrap();
                let const_999 = lose_case.add_load_value(ConstUsize::new(999));
                let [const_999] = lose_case
                    .add_dataflow_op(ConvertOpDef::ifromusize.without_log_width(), [const_999])
                    .unwrap()
                    .outputs_arr();
                lose_case.finish_with_outputs([const_999]).unwrap();

                let cond = cond_b.finish_sub_container().unwrap();

                let [cond_result] = cond.outputs_arr();

                let [usize_] = builder
                    .add_dataflow_op(ConvertOpDef::itousize.without_log_width(), [cond_result])
                    .unwrap()
                    .outputs_arr();
                builder.finish_with_outputs([usize_]).unwrap()
            })
    }

    fn add_extensions(ctx: &mut TestContext) {
        ctx.add_extensions(add_conversions_extension);
        ctx.add_extensions(add_default_prelude_extensions);
        ctx.add_extensions(add_float_extensions);
        ctx.add_extensions(add_int_extensions);
    }

    #[rstest]
    // Exact roundtrip conversion is defined on values up to 2**53 for f64.
    #[case(0)]
    #[case(3)]
    #[case(255)]
    #[case(4294967295)]
    #[case(42)]
    #[case(18_000_000_000_000_000_000)]
    fn roundtrip(mut exec_ctx: TestContext, #[case] val: u64) {
        add_extensions(&mut exec_ctx);
        let hugr = roundtrip_hugr(val);
        assert_eq!(val, exec_ctx.exec_hugr_u64(hugr, "main"));
    }

    // N.B.: There's some strange behaviour at the upper end of the ints - the
    // first case gets converted to something that's off by 1,000, but the second
    // (which is (2 ^ 64) - 1) gets converted to (2 ^ 32) - off by 9 million!
    // The fact that the first case works as expected  tells me this isn't to do
    // with int widths - maybe a floating point expert could explain that this
    // is standard behaviour...
    #[rstest]
    #[case(18_446_744_073_709_550_000, 18_446_744_073_709_549_568)]
    #[case(18_446_744_073_709_551_615,  9_223_372_036_854_775_808)] // 2 ^ 63
    fn approx_roundtrip(mut exec_ctx: TestContext, #[case] val: u64, #[case] expected: u64) {
        add_extensions(&mut exec_ctx);
        let hugr = roundtrip_hugr(val);
        assert_eq!(expected, exec_ctx.exec_hugr_u64(hugr, "main"));
    }
}