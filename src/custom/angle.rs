use anyhow::{anyhow, bail, ensure, Result};
use std::{any::TypeId, f64::consts::PI, ffi::CString};

use hugr::{
    extension::{
        prelude::{sum_with_error, ConstError, USIZE_T},
        simple_op::MakeOpDef,
        ExtensionId,
    },
    ops::{constant::CustomConst, ExtensionOp, Value},
    types::CustomType,
    HugrView,
};
use inkwell::{
    types::{BasicTypeEnum, IntType},
    values::{AsValueRef, BasicValue, BasicValueEnum, FloatValue, IntValue},
    FloatPredicate, IntPredicate,
};
use llvm_sys_140::core::LLVMBuildFreeze;

use crate::{
    emit::{emit_value, get_intrinsic, EmitFuncContext, EmitOp, EmitOpArgs},
    types::TypingSession,
};

use super::{CodegenExtension, CodegenExtsMap};

use tket2::extension::angle::{
    AngleOp, ConstAngle, ANGLE_CUSTOM_TYPE, ANGLE_EXTENSION_ID, LOG_DENOM_MAX,
};

/// A codegen extension for the `tket2.angle` extension.
///
/// We lower [ANGLE_CUSTOM_TYPE] to the same [IntType] to which [USIZE_T] is
/// lowered.  We choose to normalise all such values to have a log_denom equal
/// to the width of this type. This makes many operations simple to lower:
///  - `atrunc` becomes a no-op
///  - `aadd`, `asub`, `amul`, `aeq`, `aneg` are simply the equivalent unsigned int operations,
///    which give us the wrapping semantics we require.
///
/// As a consequence of this choice, `aparts` will always return a `log_denom`
///    of the width of [USIZE_T]. In particular this may be larger than
///    [LOG_DENOM_MAX].
///
/// The lowering of `afromrad` arbitrarily treats non-finite input (quiet NaNs,
///    +/- infinity) as zero.
///
/// We choose not to lower `adiv` as we expect it to be removed:
/// <https://github.com/CQCL/tket2/issues/605>
pub struct AngleCodegenExtension;

fn llvm_angle_type<'c, H: HugrView>(ts: &TypingSession<'c, H>) -> Result<IntType<'c>> {
    let usize_t = ts.llvm_type(&USIZE_T)?;
    ensure!(usize_t.is_int_type(), "USIZE_T is not an int type");
    Ok(usize_t.into_int_type())
}

impl<'c, H: HugrView> CodegenExtension<'c, H> for AngleCodegenExtension {
    fn extension(&self) -> ExtensionId {
        ANGLE_EXTENSION_ID
    }

    fn llvm_type(
        &self,
        context: &TypingSession<'c, H>,
        hugr_type: &CustomType,
    ) -> Result<BasicTypeEnum<'c>> {
        if hugr_type == &ANGLE_CUSTOM_TYPE {
            let r = context.llvm_type(&USIZE_T)?;
            ensure!(r.is_int_type(), "USIZE_T is not an int type");
            Ok(r)
        } else {
            bail!("Unsupported type: {hugr_type}")
        }
    }

    fn emitter<'a>(
        &'a self,
        context: &'a mut EmitFuncContext<'c, H>,
    ) -> Box<dyn crate::emit::EmitOp<'c, hugr::ops::ExtensionOp, H> + 'a> {
        Box::new(AngleOpEmitter(context))
    }

    fn supported_consts(&self) -> std::collections::HashSet<std::any::TypeId> {
        let of = TypeId::of::<ConstAngle>();
        [of].into_iter().collect()
    }

    fn load_constant(
        &self,
        context: &mut EmitFuncContext<'c, H>,
        konst: &dyn CustomConst,
    ) -> Result<Option<BasicValueEnum<'c>>> {
        let Some(angle) = konst.downcast_ref::<ConstAngle>() else {
            return Ok(None);
        };
        let angle_type = llvm_angle_type(&context.typing_session())?;
        let log_denom = angle.log_denom() as u64;
        let width = angle_type.get_bit_width() as u64;
        ensure!(
            log_denom <= width,
            "log_denom is greater than width of usize: {log_denom} > {width}"
        );
        Ok(Some(
            angle_type
                .const_int(angle.value() << (width - log_denom), false)
                .as_basic_value_enum(),
        ))
    }
}

struct AngleOpEmitter<'c, 'd, H>(&'d mut EmitFuncContext<'c, H>);

impl<'c, H: HugrView> EmitOp<'c, ExtensionOp, H> for AngleOpEmitter<'c, '_, H> {
    fn emit(&mut self, args: EmitOpArgs<'c, ExtensionOp, H>) -> Result<()> {
        let ts = self.0.typing_session();
        let module = self.0.get_current_module();
        let float_ty = self.0.iw_context().f64_type();
        let builder = self.0.builder();
        let angle_ty = llvm_angle_type(&ts)?;
        let angle_width = angle_ty.get_bit_width() as u64;

        match AngleOp::from_op(&args.node())? {
            AngleOp::atrunc => {
                // As we always normalise angles to have a log_denom of
                // angle_width, this is a no-op, and we do not need the
                // log_denom.
                let [angle, _] = args
                    .inputs
                    .try_into()
                    .map_err(|_| anyhow!("AngleOp::atrunc expects two arguments"))?;
                args.outputs.finish(builder, [angle])
            }
            AngleOp::aadd => {
                let [lhs, rhs] = args
                    .inputs
                    .try_into()
                    .map_err(|_| anyhow!("AngleOp::aadd expects two arguments"))?;
                let (lhs, rhs) = (lhs.into_int_value(), rhs.into_int_value());
                let r = builder.build_int_add(lhs, rhs, "")?;
                args.outputs.finish(builder, [r.into()])
            }
            AngleOp::asub => {
                let [lhs, rhs] = args
                    .inputs
                    .try_into()
                    .map_err(|_| anyhow!("AngleOp::asub expects one arguments"))?;
                let (lhs, rhs) = (lhs.into_int_value(), rhs.into_int_value());
                let r = builder.build_int_sub(lhs, rhs, "")?;
                args.outputs.finish(builder, [r.into()])
            }
            AngleOp::aneg => {
                let [angle] = args
                    .inputs
                    .try_into()
                    .map_err(|_| anyhow!("AngleOp::aparts expects one arguments"))?;
                let angle = angle.into_int_value();
                let r = builder.build_int_neg(angle, "")?;
                args.outputs.finish(builder, [r.into()])
            }
            AngleOp::anew => {
                let [value, log_denom] = args
                    .inputs
                    .try_into()
                    .map_err(|_| anyhow!("AngleOp::anew expects two arguments"))?;
                let value = value.into_int_value();
                let log_denom = log_denom.into_int_value();
                // this value may be poison if log_denom is too large. This is
                // accounted for below.
                let denom =
                    builder.build_left_shift(angle_ty.const_int(1, false), log_denom, "")?;
                let ok = {
                    let log_denom_ok = {
                        builder.build_int_compare(
                            IntPredicate::ULE,
                            log_denom,
                            log_denom
                                .get_type()
                                .const_int(angle_width.min(LOG_DENOM_MAX as u64), false),
                            "",
                        )?
                    };

                    let value_ok = {
                        let ok = builder.build_int_compare(IntPredicate::ULT, value, denom, "")?;
                        // if `log_denom_ok` is false, denom may be poison and hense so may `ok`.
                        // We freeze `ok` here since in this case `log_denom_ok` is false and so
                        // the `and` below will be false independently of `value_ok''s value.
                        unsafe {
                            IntValue::new(LLVMBuildFreeze(
                                builder.as_mut_ptr(),
                                ok.as_value_ref(),
                                CString::default().as_ptr(),
                            ))
                        }
                    };
                    builder.build_and(log_denom_ok, value_ok, "")?
                };
                let shift =
                    builder.build_int_sub(angle_ty.const_int(angle_width, false), log_denom, "")?;
                let value = builder.build_left_shift(value, shift, "")?;

                let ret_sum_ty = ts.llvm_sum_type(sum_with_error(USIZE_T))?;
                let success_v = ret_sum_ty.build_tag(builder, 1, vec![value.into()])?;
                let error_v = emit_value(self.0, &ConstError::new(3, "Invalid angle").into())?;
                let builder = self.0.builder();
                let failure_v = ret_sum_ty.build_tag(builder, 0, vec![error_v])?;
                let r = builder.build_select(ok, success_v, failure_v, "")?;

                args.outputs.finish(builder, [r])
            }
            AngleOp::aparts => {
                let [angle] = args
                    .inputs
                    .try_into()
                    .map_err(|_| anyhow!("AngleOp::aparts expects one argument"))?;
                args.outputs.finish(
                    builder,
                    [angle, angle_ty.const_int(angle_width, false).into()],
                )
            }
            AngleOp::afromrad => {
                // As we always normalise angles to have a log_denom of
                // angle_width, we do not need the log_denom.
                let [_log_denom, rads] = args
                    .inputs
                    .try_into()
                    .map_err(|_| anyhow!("AngleOp::afromrad expects two arguments"))?;
                let rads: FloatValue<'c> = rads
                    .try_into()
                    .map_err(|_| anyhow!("afromrad expects a float argument"))?;
                let float_ty = rads.get_type();
                let two_pi = float_ty.const_float(PI * 2.0);
                // normalised_rads is in the interval 0..1
                let normalised_rads = {
                    // normalised_rads = (rads / (2 * PI)) - floor(rads / (2 * PI))
                    // note that floor(x) gives the smallest integral value less than x
                    // so this deals with both positive and negative rads

                    let rads_by_2pi = builder.build_float_div(rads, two_pi, "")?;
                    let floor_rads_by_2pi = {
                        let floor = get_intrinsic(module, "llvm.floor", [float_ty.into()])?;
                        builder
                            .build_call(floor, &[rads_by_2pi.into()], "")?
                            .try_as_basic_value()
                            .left()
                            .ok_or(anyhow!("llvm.floor has no return value"))?
                            .into_float_value()
                    };

                    let normalised_rads =
                        builder.build_float_sub(rads_by_2pi, floor_rads_by_2pi, "")?;

                    // We choose to treat {Quiet NaNs, infinities} as zero.
                    // the `llvm.is.fpclass` intrinsic was introduced in llvm 15
                    // and is the best way to distinguish these float values.
                    // For now we are using llvm 14, and so we use 3 `feq`s.
                    // Below is commented code that we can use once we support
                    // llvm 15.
                    #[cfg(feature = "llvm14-0")]
                    let rads_ok = {
                        let is_pos_inf = builder.build_float_compare(
                            FloatPredicate::OEQ,
                            rads,
                            float_ty.const_float(f64::INFINITY),
                            "",
                        )?;
                        let is_neg_inf = builder.build_float_compare(
                            FloatPredicate::OEQ,
                            rads,
                            float_ty.const_float(f64::NEG_INFINITY),
                            "",
                        )?;
                        let is_nan = builder.build_float_compare(
                            FloatPredicate::UNO,
                            rads,
                            float_ty.const_zero(),
                            "",
                        )?;
                        let r = builder.build_or(is_pos_inf, is_neg_inf, "")?;
                        let r = builder.build_or(r, is_nan, "")?;
                        builder.build_not(r, "")?
                    };
                    // let rads_ok = {
                    //     let i32_ty = self.0.iw_context().i32_type();
                    //     let builder = self.0.builder();
                    //     let is_fpclass = get_intrinsic(module, "llvm.is.fpclass", [float_ty.as_basic_type_enum(), i32_ty.as_basic_type_enum()])?;
                    //     // Here we pick out the following floats:
                    //     //  - bit 0: Signalling Nan
                    //     //  - bit 3: Negative normal
                    //     //  - bit 8: Positive normal
                    //     let test = i32_ty.const_int((1 << 0) | (1 << 3) | (1 << 8), false);
                    //     builder
                    //         .build_call(is_fpclass, &[rads.into(), test.into()], "")?
                    //         .try_as_basic_value()
                    //         .left()
                    //         .ok_or(anyhow!("llvm.is.fpclass has no return value"))?
                    //         .into_int_value()
                    // };
                    let zero = float_ty.const_zero();
                    builder
                        .build_select(rads_ok, normalised_rads, zero, "")?
                        .into_float_value()
                };

                let value = {
                    // value = int(normalised_value * 2 ^ angle_width + .5)
                    let exp2 = get_intrinsic(module, "llvm.exp2", [float_ty.into()])?;
                    let log_denom = float_ty.const_float(angle_width as f64);
                    let denom = builder
                        .build_call(exp2, &[log_denom.into()], "")?
                        .try_as_basic_value()
                        .left()
                        .ok_or(anyhow!("exp2 intrinsic had no return value"))?
                        .into_float_value();
                    builder.build_float_to_unsigned_int(
                        builder.build_float_add(
                            builder.build_float_mul(normalised_rads, denom, "")?,
                            float_ty.const_float(0.5),
                            "",
                        )?,
                        angle_ty,
                        "",
                    )?
                };
                args.outputs.finish(builder, [value.into()])
            }
            AngleOp::atorad => {
                let [angle] = args
                    .inputs
                    .try_into()
                    .map_err(|_| anyhow!("AngleOp::atorad expects one arguments"))?;
                let angle = angle.into_int_value();
                let r = {
                    // r = angle * 2 * PI / 2 ^ angle_width = angle * PI * 2 ^ -(angle_width - 1)
                    let value = builder.build_unsigned_int_to_float(angle, float_ty, "")?;
                    let denom = {
                        let exp2 = get_intrinsic(module, "llvm.exp2", [float_ty.into()])?;
                        builder
                            .build_call(
                                exp2,
                                &[float_ty.const_float(-((angle_width - 1) as f64)).into()],
                                "",
                            )?
                            .try_as_basic_value()
                            .left()
                            .ok_or(anyhow!("exp2 intrinsic had no return value"))?
                            .into_float_value()
                    };
                    let value = builder.build_float_mul(value, float_ty.const_float(PI), "")?;
                    builder.build_float_mul(value, denom, "")?
                };
                args.outputs.finish(builder, [r.into()])
            }
            AngleOp::aeq => {
                let [lhs, rhs] = args
                    .inputs
                    .try_into()
                    .map_err(|_| anyhow!("AngleOp::aeq expects two arguments"))?;
                let (lhs, rhs) = (lhs.into_int_value(), rhs.into_int_value());
                let r = {
                    let r_i1 = builder.build_int_compare(IntPredicate::EQ, lhs, rhs, "")?;
                    let true_val = emit_value(self.0, &Value::true_val())?;
                    let false_val = emit_value(self.0, &Value::false_val())?;
                    self.0
                        .builder()
                        .build_select(r_i1, true_val, false_val, "")?
                };
                args.outputs.finish(self.0.builder(), [r])
            }
            AngleOp::amul => {
                let [lhs, rhs] = args
                    .inputs
                    .try_into()
                    .map_err(|_| anyhow!("AngleOp::amul expects two arguments"))?;
                let (lhs, rhs) = (lhs.into_int_value(), rhs.into_int_value());
                let r = builder.build_int_mul(lhs, rhs, "")?;
                args.outputs.finish(builder, [r.into()])
            }
            op => bail!("Unsupported op: {op:?}"),
        }
    }
}

pub fn add_angle_extensions<H: HugrView>(cge: CodegenExtsMap<'_, H>) -> CodegenExtsMap<'_, H> {
    cge.add_cge(AngleCodegenExtension)
}

impl<'c, H: HugrView> CodegenExtsMap<'c, H> {
    pub fn add_angle_extensions(self) -> Self {
        add_angle_extensions(self)
    }
}

#[cfg(test)]
mod test {
    use std::collections::HashSet;

    use hugr::extension::prelude::ConstUsize;
    use hugr::{
        builder::{Dataflow, DataflowSubContainer as _, SubContainer},
        extension::{prelude::BOOL_T, ExtensionSet},
        ops::OpName,
        std_extensions::arithmetic::float_types::{self, ConstF64, FLOAT64_TYPE},
    };
    use rstest::rstest;
    use tket2::extension::angle::{AngleOpBuilder as _, ANGLE_TYPE};

    use crate::{
        check_emission,
        emit::test::SimpleHugrConfig,
        test::{exec_ctx, llvm_ctx, TestContext},
        types::HugrType,
    };

    use super::*;

    #[rstest]
    fn emit_all_ops(mut llvm_ctx: TestContext) {
        let hugr = SimpleHugrConfig::new()
            .with_ins(vec![ANGLE_TYPE, USIZE_T])
            .with_outs(BOOL_T)
            .with_extensions(tket2::extension::REGISTRY.to_owned())
            .finish(|mut builder| {
                let [angle, scalar] = builder.input_wires_arr();
                let radians = builder.add_atorad(angle).unwrap();
                let angle = builder.add_afromrad(scalar, radians).unwrap();
                let angle = builder.add_amul(angle, scalar).unwrap();
                // let angle = builder.add_adiv(angle, scalar).unwrap();
                let angle = builder.add_aadd(angle, angle).unwrap();
                let angle = builder.add_asub(angle, angle).unwrap();
                let [num, log_denom] = builder.add_aparts(angle).unwrap();
                let _angle_sum = builder.add_anew(num, log_denom).unwrap();
                let angle = builder.add_aneg(angle).unwrap();
                let angle = builder.add_atrunc(angle, log_denom).unwrap();
                let bool = builder.add_aeq(angle, angle).unwrap();
                builder.finish_with_outputs([bool]).unwrap()
            });
        llvm_ctx.add_extensions(|cge| {
            cge.add_angle_extensions()
                .add_default_prelude_extensions()
                .add_float_extensions()
        });
        check_emission!(hugr, llvm_ctx);
    }

    #[rstest]
    #[case(1,1, 1 << 63)]
    #[case(0, 1, 0)]
    #[case(3, 1, 0)]
    #[case(8,4, 1 << 63)]
    fn exec_anew(
        mut exec_ctx: TestContext,
        #[case] value: u64,
        #[case] log_denom: u8,
        #[case] expected_aparts_value: u64,
    ) {
        let hugr = SimpleHugrConfig::new()
            .with_extensions(tket2::extension::REGISTRY.to_owned())
            .with_outs(USIZE_T)
            .finish(|mut builder| {
                let value = builder.add_load_value(ConstUsize::new(value));
                let log_denom = builder.add_load_value(ConstUsize::new(log_denom as u64));
                let mb_angle = builder.add_anew(value, log_denom).unwrap();
                let r = {
                    let variants = {
                        let et = sum_with_error(ANGLE_TYPE);
                        (0..2)
                            .map(|i| et.get_variant(i).unwrap().clone().try_into().unwrap())
                            .collect::<Vec<_>>()
                    };
                    let mut conditional = builder
                        .conditional_builder((variants, mb_angle), [], USIZE_T.into())
                        .unwrap();
                    {
                        let mut case = conditional.case_builder(0).unwrap();
                        let us0 = case.add_load_value(ConstUsize::new(0));
                        case.finish_with_outputs([us0]).unwrap();
                    }
                    {
                        let mut case = conditional.case_builder(1).unwrap();
                        let [angle] = case.input_wires_arr();
                        let [value, _log_denom] = case.add_aparts(angle).unwrap();
                        case.finish_with_outputs([value]).unwrap();
                    }
                    conditional.finish_sub_container().unwrap().out_wire(0)
                };
                builder.finish_with_outputs([r]).unwrap()
            });
        exec_ctx.add_extensions(|cge| cge.add_angle_extensions().add_default_prelude_extensions());

        assert_eq!(expected_aparts_value, exec_ctx.exec_hugr_u64(hugr, "main"));
    }

    #[rstest]
    #[case(ConstAngle::PI, 1, 1 << 63)]
    #[case(ConstAngle::PI, LOG_DENOM_MAX, 1 << 63)]
    fn exec_atrunc(
        mut exec_ctx: TestContext,
        #[case] angle: ConstAngle,
        #[case] log_denom: u8,
        #[case] expected_aparts_value: u64,
    ) {
        let hugr = SimpleHugrConfig::new()
            .with_extensions(tket2::extension::REGISTRY.to_owned())
            .with_outs(USIZE_T)
            .finish(|mut builder| {
                let angle = builder.add_load_value(angle);
                let log_denom = builder.add_load_value(ConstUsize::new(log_denom as u64));
                let angle = builder.add_atrunc(angle, log_denom).unwrap();
                let [value, _log_denom] = builder.add_aparts(angle).unwrap();
                builder.finish_with_outputs([value]).unwrap()
            });
        exec_ctx.add_extensions(|cge| cge.add_angle_extensions().add_default_prelude_extensions());

        assert_eq!(expected_aparts_value, exec_ctx.exec_hugr_u64(hugr, "main"));
    }

    #[rstest]
    #[case(ConstAngle::new(1, 1).unwrap(), ConstAngle::new(4, 4).unwrap(), 3 << 62)]
    #[case(ConstAngle::PI, ConstAngle::new(4, 8).unwrap(), 0)]
    fn exec_aadd(
        mut exec_ctx: TestContext,
        #[case] angle1: ConstAngle,
        #[case] angle2: ConstAngle,
        #[case] expected_aparts_value: u64,
    ) {
        let hugr = SimpleHugrConfig::new()
            .with_extensions(tket2::extension::REGISTRY.to_owned())
            .with_outs(USIZE_T)
            .finish(|mut builder| {
                let angle1 = builder.add_load_value(angle1);
                let angle2 = builder.add_load_value(angle2);
                let angle = builder.add_aadd(angle1, angle2).unwrap();
                let [value, _log_denom] = builder.add_aparts(angle).unwrap();
                builder.finish_with_outputs([value]).unwrap()
            });
        exec_ctx.add_extensions(|cge| cge.add_angle_extensions().add_default_prelude_extensions());

        assert_eq!(expected_aparts_value, exec_ctx.exec_hugr_u64(hugr, "main"));
    }

    #[rstest]
    #[case(ConstAngle::new(1, 1).unwrap(), ConstAngle::new(4, 4).unwrap(), 1 << 62)]
    #[case(ConstAngle::PI, ConstAngle::new(4, 8).unwrap(), 0)]
    fn exec_asub(
        mut exec_ctx: TestContext,
        #[case] angle1: ConstAngle,
        #[case] angle2: ConstAngle,
        #[case] expected_aparts_value: u64,
    ) {
        let hugr = SimpleHugrConfig::new()
            .with_extensions(tket2::extension::REGISTRY.to_owned())
            .with_outs(USIZE_T)
            .finish(|mut builder| {
                let angle1 = builder.add_load_value(angle1);
                let angle2 = builder.add_load_value(angle2);
                let angle = builder.add_asub(angle1, angle2).unwrap();
                let [value, _log_denom] = builder.add_aparts(angle).unwrap();
                builder.finish_with_outputs([value]).unwrap()
            });
        exec_ctx.add_extensions(|cge| cge.add_angle_extensions().add_default_prelude_extensions());

        assert_eq!(expected_aparts_value, exec_ctx.exec_hugr_u64(hugr, "main"));
    }

    #[rstest]
    #[case(ConstAngle::PI, 2, 0)]
    #[case(ConstAngle::PI, 3, 1 << 63)]
    #[case(ConstAngle::PI, 11, 1 << 63)]
    fn exec_amul(
        mut exec_ctx: TestContext,
        #[case] angle: ConstAngle,
        #[case] factor: u64,
        #[case] expected_aparts_value: u64,
    ) {
        let hugr = SimpleHugrConfig::new()
            .with_extensions(tket2::extension::REGISTRY.to_owned())
            .with_outs(USIZE_T)
            .finish(|mut builder| {
                let angle = builder.add_load_value(angle);
                let factor = builder.add_load_value(ConstUsize::new(factor));
                let angle = builder.add_amul(angle, factor).unwrap();
                let [value, _log_denom] = builder.add_aparts(angle).unwrap();
                builder.finish_with_outputs([value]).unwrap()
            });
        exec_ctx.add_extensions(|cge| cge.add_angle_extensions().add_default_prelude_extensions());

        assert_eq!(expected_aparts_value, exec_ctx.exec_hugr_u64(hugr, "main"));
    }

    #[rstest]
    #[case(ConstAngle::PI, 1 << 63)]
    #[case(ConstAngle::PI_2, 3 << 62)]
    #[case(ConstAngle::PI_4, 7 << 61)]
    fn exec_aneg(
        mut exec_ctx: TestContext,
        #[case] angle: ConstAngle,
        #[case] expected_aparts_value: u64,
    ) {
        let hugr = SimpleHugrConfig::new()
            .with_extensions(tket2::extension::REGISTRY.to_owned())
            .with_outs(USIZE_T)
            .finish(|mut builder| {
                let angle = builder.add_load_value(angle);
                let angle = builder.add_aneg(angle).unwrap();
                let [value, _log_denom] = builder.add_aparts(angle).unwrap();
                builder.finish_with_outputs([value]).unwrap()
            });
        exec_ctx.add_extensions(|cge| cge.add_angle_extensions().add_default_prelude_extensions());

        assert_eq!(expected_aparts_value, exec_ctx.exec_hugr_u64(hugr, "main"));
    }

    #[rstest]
    #[case(ConstAngle::PI, PI)]
    // #[case(ConstAngle::TAU, 2.0 * PI)]
    #[case(ConstAngle::PI_2, PI / 2.0)]
    #[case(ConstAngle::PI_4, PI / 4.0)]
    fn exec_atorad(
        mut exec_ctx: TestContext,
        #[case] angle: ConstAngle,
        #[case] expected_rads: f64,
    ) {
        let hugr = SimpleHugrConfig::new()
            .with_extensions(tket2::extension::REGISTRY.to_owned())
            .with_outs(FLOAT64_TYPE)
            .finish(|mut builder| {
                let angle = builder.add_load_value(angle);
                let rads = builder.add_atorad(angle).unwrap();
                builder.finish_with_outputs([rads]).unwrap()
            });
        exec_ctx.add_extensions(|cge| {
            cge.add_angle_extensions()
                .add_default_prelude_extensions()
                .add_float_extensions()
        });

        let rads = exec_ctx.exec_hugr_f64(hugr, "main");
        let epsilon = 0.0000000000001; // chosen without too much thought
        assert!(f64::abs(expected_rads - rads) < epsilon);
    }

    #[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
    struct NonFiniteConst64(f64);

    #[typetag::serde]
    impl CustomConst for NonFiniteConst64 {
        fn name(&self) -> OpName {
            "NonFiniteConst64".into()
        }

        fn extension_reqs(&self) -> ExtensionSet {
            float_types::EXTENSION_ID.into()
        }

        fn get_type(&self) -> HugrType {
            FLOAT64_TYPE
        }
    }

    struct NonFiniteConst64CodegenExtension;

    impl<'c, H: HugrView> CodegenExtension<'c, H> for NonFiniteConst64CodegenExtension {
        fn extension(&self) -> ExtensionId {
            ExtensionId::new_unchecked("NonFiniteConst64")
        }

        fn llvm_type(&self, _: &TypingSession<'c, H>, _: &CustomType) -> Result<BasicTypeEnum<'c>> {
            panic!("no types")
        }

        fn emitter<'a>(
            &'a self,
            _: &'a mut EmitFuncContext<'c, H>,
        ) -> Box<dyn EmitOp<'c, ExtensionOp, H> + 'a> {
            panic!("no ops")
        }

        fn supported_consts(&self) -> HashSet<TypeId> {
            let of = TypeId::of::<NonFiniteConst64>();
            [of].into_iter().collect()
        }

        fn load_constant(
            &self,
            context: &mut EmitFuncContext<'c, H>,
            konst: &dyn CustomConst,
        ) -> Result<Option<BasicValueEnum<'c>>> {
            let Some(NonFiniteConst64(f)) = konst.downcast_ref::<NonFiniteConst64>() else {
                panic!("load_constant")
            };
            Ok(Some(context.iw_context().f64_type().const_float(*f).into()))
        }
    }

    #[rstest]
    #[case(PI, 1<<63)]
    #[case(-PI, 1<<63)]
    // #[case(ConstAngle::TAU, 2.0 * PI)]
    #[case(PI / 2.0, 1 << 62)]
    #[case(-PI / 2.0, 3 << 62)]
    #[case(PI / 4.0, 1 << 61)]
    #[case(-PI / 4.0, 7 << 61)]
    #[case(13.0 * PI, 1 << 63)]
    #[case(-13.0 * PI, 1 << 63)]
    #[case(f64::NAN, 0)]
    #[case(f64::INFINITY, 0)]
    #[case(f64::NEG_INFINITY, 0)]
    fn exec_afromrad(
        mut exec_ctx: TestContext,
        #[case] rads: f64,
        #[case] expected_aparts_value: u64,
    ) {
        let hugr = SimpleHugrConfig::new()
            .with_extensions(tket2::extension::REGISTRY.to_owned())
            .with_outs(USIZE_T)
            .finish(|mut builder| {
                let konst: Value = if rads.is_finite() {
                    ConstF64::new(rads).into()
                } else {
                    NonFiniteConst64(rads).into()
                };
                let rads = builder.add_load_value(konst);
                let us4 = builder.add_load_value(ConstUsize::new(4));
                let angle = builder.add_afromrad(us4, rads).unwrap();
                let [value, _log_denom] = builder.add_aparts(angle).unwrap();
                builder.finish_with_outputs([value]).unwrap()
            });
        exec_ctx.add_extensions(|cge| {
            cge.add_angle_extensions()
                .add_default_prelude_extensions()
                .add_float_extensions()
                .add_cge(NonFiniteConst64CodegenExtension)
        });

        let r = exec_ctx.exec_hugr_u64(hugr, "main");
        // chosen without too much thought, except that a f64 has 53 bits of
        // precision so 1 << 11 is the lowest reasonable value.
        let epsilon = 1 << 15;
        assert!((expected_aparts_value.wrapping_sub(r) as i64).abs() < epsilon);
    }
}
