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
    types::BasicTypeEnum,
    values::{AsValueRef, BasicValue, BasicValueEnum, FloatValue, IntValue},
    IntPredicate,
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

// #[derive(Debug, Clone, Copy)]
// struct LLVMAngleType<'c>(IntType<'c>);

// impl<'c> LLVMAngleType<'c> {
//     pub fn new(usize_type: IntType<'c>) -> Self {
//         Self(usize_type)
//     }

//     fn value_field_type(&self) -> IntType<'c> {
//         self.0
//     }

//     pub fn const_angle(&self, value: u64, log_denom: u8) -> Result<LLVMAngleValue<'c>> {
//         let log_denom = log_denom as u64;
//         let width =
//             self.0.size_of().get_zero_extended_constant().ok_or(anyhow!("Width of
//             usize is not a constant"))?;
//         ensure!(width >= log_denom, "log_denom is greater than width of usize: {log_denom} > {width}");
//         Ok(LLVMAngleValue(self.0.const_int(value << (width - log_denom), false), *self))
//     }

//     // pub fn build_value(
//     //     &self,
//     //     builder: &Builder<'c>,
//     //     value: impl BasicValue<'c>,
//     //     log_denom: impl BasicValue<'c>,
//     // ) -> Result<LLVMAngleValue<'c>> {
//     //     let (value, log_denom) = (value.as_basic_value_enum(), log_denom.as_basic_value_enum());
//     //     ensure!(value.get_type() == self.value_field_type().as_basic_type_enum());

//     //     let r = self.0.get_undef();
//     //     let r = builder.build_insert_value(r, value, 0, "")?;
//     //     let r = builder.build_insert_value(r, log_denom, 1, "")?;
//     //     Ok(LLVMAngleValue(r.into_struct_value(), *self))
//     // }
// }

// unsafe impl<'c> AsTypeRef for LLVMAngleType<'c> {
//     fn as_type_ref(&self) -> LLVMTypeRef {
//         self.0.as_type_ref()
//     }
// }

// unsafe impl<'c> AnyType<'c> for LLVMAngleType<'c> {}
// unsafe impl<'c> BasicType<'c> for LLVMAngleType<'c> {}

// #[derive(Debug, Clone, Copy)]
// struct LLVMAngleValue<'c>(IntValue<'c>, LLVMAngleType<'c>);

// impl<'c> LLVMAngleValue<'c> {
//     fn try_new(typ: LLVMAngleType<'c>, value: impl BasicValue<'c>) -> Result<Self> {
//         let value = value.as_basic_value_enum();
//         ensure!(typ.as_basic_type_enum() == value.get_type());
//         Ok(Self(value.into_int_value(), typ))
//     }

//     fn build_get_value(&self, _builder: &Builder<'c>) -> Result<IntValue<'c>> {
//         Ok(self.0)
//     }
// }

// impl<'c> From<LLVMAngleValue<'c>> for BasicValueEnum<'c> {
//     fn from(value: LLVMAngleValue<'c>) -> Self {
//         value.as_basic_value_enum()
//     }
// }

// unsafe impl<'c> AsValueRef for LLVMAngleValue<'c> {
//     fn as_value_ref(&self) -> LLVMValueRef {
//         self.0.as_value_ref()
//     }
// }

// unsafe impl<'c> AnyValue<'c> for LLVMAngleValue<'c> {}
// unsafe impl<'c> BasicValue<'c> for LLVMAngleValue<'c> {}

pub struct AngleCodegenExtension;

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
        let angle_type = context.llvm_type(&USIZE_T)?.into_int_type();
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

// impl<'c, 'd, H: HugrView> AngleOpEmitter<'c, 'd, H> {
//     fn binary_angle_op<E>(
//         &self,
//         lhs: LLVMAngleValue<'c>,
//         rhs: LLVMAngleValue<'c>,
//         go: impl FnOnce(IntValue<'c>, IntValue<'c>) -> Result<IntValue<'c>, E>,
//     ) -> Result<LLVMAngleValue<'c>>
//     where
//         anyhow::Error: From<E>,
//     {
//         let angle_ty = self.1;
//         let builder = self.0.builder();
//         let lhs_value = lhs.build_get_value(builder)?;
//         let rhs_value = lhs.build_get_value(builder)?;
//         let new_value = go(lhs_value, rhs_value)?;

//         let lhs_log_denom = lhs.build_get_log_denom(builder)?;
//         let rhs_log_denom = lhs.build_get_log_denom(builder)?;

//         let lhs_log_denom_larger =
//             builder.build_int_compare(IntPredicate::UGT, lhs_log_denom, rhs_log_denom, "")?;
//         let lhs_larger_r = {
//             let v = lhs.build_unmax_denom(builder, new_value)?;
//             angle_ty.build_value(builder, v, lhs_log_denom)?
//         };
//         let rhs_larger_r = {
//             let v = rhs.build_unmax_denom(builder, new_value)?;
//             angle_ty.build_value(builder, v, rhs_log_denom)?
//         };
//         let r = builder.build_select(lhs_log_denom_larger, lhs_larger_r, rhs_larger_r, "")?;
//         LLVMAngleValue::try_new(angle_ty, r)
//     }
// }

impl<'c, H: HugrView> EmitOp<'c, ExtensionOp, H> for AngleOpEmitter<'c, '_, H> {
    fn emit(&mut self, args: EmitOpArgs<'c, ExtensionOp, H>) -> Result<()> {
        let ts = self.0.typing_session();
        let module = self.0.get_current_module();
        let float_ty = self.0.iw_context().f64_type();
        let builder = self.0.builder();
        let angle_ty = self.0.llvm_type(&USIZE_T)?.into_int_type();
        let angle_width = angle_ty.get_bit_width() as u64;

        match AngleOp::from_op(&args.node())? {
            AngleOp::atrunc => {
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
                let denom =
                    builder.build_left_shift(angle_ty.const_int(1, false), log_denom, "")?;
                let ok = {
                    let log_denom_ok = {
                        let log_denom_in_range = builder.build_int_compare(
                            IntPredicate::ULE,
                            log_denom,
                            log_denom.get_type().const_int(LOG_DENOM_MAX as u64, false),
                            "",
                        )?;
                        let width_large_enough = builder.build_int_compare(
                            IntPredicate::ULE,
                            log_denom,
                            angle_ty.const_int(angle_width, false),
                            "",
                        )?;
                        builder.build_and(log_denom_in_range, width_large_enough, "")?
                    };

                    let value_ok = {
                        let ok = builder.build_int_compare(IntPredicate::ULT, value, denom, "")?;
                        // if `log_denom_ok` is false, denom may be poison and so may `ok`.
                        // We freeze `ok` here since `log_denom_ok` is false and so
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
                let [_log_denom, rads] = args
                    .inputs
                    .try_into()
                    .map_err(|_| anyhow!("AngleOp::afromrad expects two arguments"))?;
                let rads: FloatValue<'c> = rads
                    .try_into()
                    .map_err(|_| anyhow!("afromrad expects a float argument"))?;
                let float_ty = rads.get_type();
                let two_pi = float_ty.const_float(PI * 2.0);
                // normalised_rads will be in the interval 0..1
                let normalised_rads = {
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

                    builder.build_float_sub(rads_by_2pi, floor_rads_by_2pi, "")?
                    // let rads_ok = {
                    //     let is_fpclass = get_intrinsic(module, "llvm.is.fpclass", [float_ty.as_basic_type_enum(), i32_ty.as_basic_type_enum()])?;
                    //     // We choose to treat {Quiet NaNs, infinities, subnormal values} as zero.
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
                    // let zero = float_ty.const_zero();
                    // builder.build_select(rads_ok, normalised_rads, zero, "")?.into_float_value()
                };

                let value = {
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
                    let value = builder.build_unsigned_int_to_float(angle, float_ty, "")?;
                    let denom = {
                        let exp2 = get_intrinsic(module, "llvm.exp2", [float_ty.into()])?;
                        builder
                            .build_call(
                                exp2,
                                &[float_ty.const_float(angle_width as f64).into()],
                                "",
                            )?
                            .try_as_basic_value()
                            .left()
                            .ok_or(anyhow!("exp2 intrinsic had no return value"))?
                            .into_float_value()
                    };
                    let value =
                        builder.build_float_mul(value, float_ty.const_float(PI * 2.0), "")?;
                    builder.build_float_div(value, denom, "")?
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
            AngleOp::adiv => {
                let [lhs, rhs] = args
                    .inputs
                    .try_into()
                    .map_err(|_| anyhow!("AngleOp::adiv expects two arguments"))?;
                let (lhs, rhs) = (lhs.into_int_value(), rhs.into_int_value());
                // Division by zero is undefined behaviour in LLVM. Should we:
                //  - leave this as is. I.e. it is undefined behaviour in HUGR
                //  - check for zero and branch, then in the is-zero branch:
                //    - panic
                //    - return poison. I.e. it is fine in HUGR if you never look at the result
                let r = builder.build_int_unsigned_div(lhs, rhs, "")?;
                args.outputs.finish(builder, [r.into()])
            }
            _ => todo!(),
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
    use hugr::{
        builder::{Dataflow as _, DataflowSubContainer as _},
        extension::prelude::BOOL_T,
    };
    use rstest::rstest;
    use tket2::extension::angle::{AngleOpBuilder as _, ANGLE_TYPE};

    use crate::{
        check_emission,
        emit::test::SimpleHugrConfig,
        test::{llvm_ctx, TestContext},
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
}
