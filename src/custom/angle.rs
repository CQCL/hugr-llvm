use anyhow::{anyhow, bail, ensure, Result};
use std::{any::TypeId, char::decode_utf16, f64::consts::PI};

use hugr::{
    extension::{prelude::ConstUsize, simple_op::MakeOpDef, ExtensionId},
    ops::{constant::CustomConst, ExtensionOp, Value},
    std_extensions::arithmetic::int_types::{self, int_type, ConstInt},
    types::{CustomType, SumType},
    HugrView,
};
use inkwell::{
    builder::Builder,
    context::Context,
    intrinsics::Intrinsic,
    types::{AnyType, AsTypeRef, BasicType, BasicTypeEnum, IntType, StructType},
    values::{AnyValue, AsValueRef, BasicValue, BasicValueEnum, FloatValue, IntValue, StructValue},
    FloatPredicate, IntPredicate,
};
use llvm_sys_140::prelude::{LLVMTypeRef, LLVMValueRef};

use crate::{
    emit::{emit_value, EmitFuncContext, EmitOp, EmitOpArgs},
    sum::LLVMSumType,
    types::TypingSession,
};

use super::CodegenExtension;

use tket2::extension::angle::{AngleOp, ConstAngle, ANGLE_CUSTOM_TYPE, ANGLE_EXTENSION_ID};

#[derive(Debug, Clone, Copy)]
struct LLVMAngleType<'c>(StructType<'c>);

impl<'c> LLVMAngleType<'c> {
    pub fn new(context: &'c Context, usize_type: IntType<'c>) -> Self {
        Self(context.struct_type(&[usize_type.into(), usize_type.into()], false))
    }

    fn value_field_type(&self) -> IntType<'c> {
        assert_eq!(2, self.0.get_field_types().len());
        unsafe { self.0.get_field_type_at_index_unchecked(0) }.into_int_type()
    }

    fn log_denom_field_type(&self) -> IntType<'c> {
        assert_eq!(2, self.0.get_field_types().len());
        unsafe { self.0.get_field_type_at_index_unchecked(1) }.into_int_type()
    }

    pub fn const_angle(&self, value: u64, log_denom: u8) -> LLVMAngleValue<'c> {
        assert_eq!(2, self.0.get_field_types().len());
        let v = self.0.get_undef();
        v.set_field_at_index(0, self.value_field_type().const_int(value, false));
        v.set_field_at_index(
            1,
            self.log_denom_field_type()
                .const_int(log_denom as u64, false),
        );
        LLVMAngleValue(v, *self)
    }

    pub fn build_value(
        &self,
        builder: &Builder<'c>,
        value: impl BasicValue<'c>,
        log_denom: impl BasicValue<'c>,
    ) -> Result<LLVMAngleValue<'c>> {
        let (value, log_denom) = (value.as_basic_value_enum(), log_denom.as_basic_value_enum());
        ensure!(value.get_type() == self.value_field_type().as_basic_type_enum());
        ensure!(log_denom.get_type() == self.log_denom_field_type().as_basic_type_enum());

        let r = self.0.get_undef();
        let r = builder.build_insert_value(r, value, 0, "")?;
        let r = builder.build_insert_value(r, log_denom, 1, "")?;
        Ok(LLVMAngleValue(r.into_struct_value(), *self))
    }
}

unsafe impl<'c> AsTypeRef for LLVMAngleType<'c> {
    fn as_type_ref(&self) -> LLVMTypeRef {
        self.0.as_type_ref()
    }
}

unsafe impl<'c> AnyType<'c> for LLVMAngleType<'c> {}
unsafe impl<'c> BasicType<'c> for LLVMAngleType<'c> {}

#[derive(Debug, Clone, Copy)]
struct LLVMAngleValue<'c>(StructValue<'c>, LLVMAngleType<'c>);

impl<'c> LLVMAngleValue<'c> {
    fn try_new(typ: LLVMAngleType<'c>, value: impl BasicValue<'c>) -> Result<Self> {
        let value = value.as_basic_value_enum();
        ensure!(typ.as_basic_type_enum() == value.get_type());
        Ok(Self(value.into_struct_value(), typ))
    }

    fn build_get_value(&self, builder: &Builder<'c>) -> Result<IntValue<'c>> {
        Ok(builder.build_extract_value(self.0, 0, "")?.into_int_value())
    }

    fn build_get_log_denom(&self, builder: &Builder<'c>) -> Result<IntValue<'c>> {
        Ok(builder.build_extract_value(self.0, 1, "")?.into_int_value())
    }

    fn build_unmax_denom(
        &self,
        builder: &Builder<'c>,
        max_denom_value: IntValue<'c>,
    ) -> Result<IntValue<'c>> {
        let log_denom = self.build_get_log_denom(builder)?;
        let shift = builder.build_int_sub(self.1.value_field_type().size_of(), log_denom, "")?;
        Ok(builder.build_right_shift(max_denom_value, shift, false, "")?)
    }
    fn build_get_value_max_denom(&self, builder: &Builder<'c>) -> Result<IntValue<'c>> {
        let value = self.build_get_value(builder)?;
        let log_denom = self.build_get_log_denom(builder)?;
        let shift = builder.build_int_sub(self.1.value_field_type().size_of(), log_denom, "")?;
        Ok(builder.build_left_shift(value, shift, "")?)
    }
}

impl<'c> From<LLVMAngleValue<'c>> for BasicValueEnum<'c> {
    fn from(value: LLVMAngleValue<'c>) -> Self {
        value.as_basic_value_enum()
    }
}

unsafe impl<'c> AsValueRef for LLVMAngleValue<'c> {
    fn as_value_ref(&self) -> LLVMValueRef {
        self.0.as_value_ref()
    }
}

unsafe impl<'c> AnyValue<'c> for LLVMAngleValue<'c> {}
unsafe impl<'c> BasicValue<'c> for LLVMAngleValue<'c> {}

pub struct AngleCodegenExtension<'c> {
    usize_type: IntType<'c>,
}

impl<'c> AngleCodegenExtension<'c> {
    fn angle_type(&self, context: &'c Context) -> LLVMAngleType<'c> {
        LLVMAngleType::new(context, self.usize_type)
    }
}

impl<'c, H: HugrView> CodegenExtension<'c, H> for AngleCodegenExtension<'c> {
    fn extension(&self) -> ExtensionId {
        ANGLE_EXTENSION_ID
    }

    fn llvm_type(
        &self,
        context: &TypingSession<'c, H>,
        hugr_type: &CustomType,
    ) -> Result<BasicTypeEnum<'c>> {
        if hugr_type == &ANGLE_CUSTOM_TYPE {
            Ok(self.angle_type(context.iw_context()).as_basic_type_enum())
        } else {
            bail!("Unsupported type: {hugr_type}")
        }
    }

    fn emitter<'a>(
        &'a self,
        context: &'a mut EmitFuncContext<'c, H>,
    ) -> Box<dyn crate::emit::EmitOp<'c, hugr::ops::ExtensionOp, H> + 'a> {
        Box::new(AngleOpEmitter(
            context,
            self.angle_type(context.iw_context()),
        ))
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
        let angle_type = self.angle_type(context.iw_context());
        Ok(Some(
            angle_type
                .const_angle(angle.value(), angle.log_denom())
                .as_basic_value_enum(),
        ))
    }
}

struct AngleOpEmitter<'c, 'd, H>(&'d mut EmitFuncContext<'c, H>, LLVMAngleType<'c>);

impl<'c, 'd, H: HugrView> AngleOpEmitter<'c, 'd, H> {
    fn binary_angle_op<E>(
        &self,
        lhs: LLVMAngleValue<'c>,
        rhs: LLVMAngleValue<'c>,
        go: impl FnOnce(IntValue<'c>, IntValue<'c>) -> Result<IntValue<'c>, E>,
    ) -> Result<LLVMAngleValue<'c>>
    where
        anyhow::Error: From<E>,
    {
        let angle_ty = self.1;
        let builder = self.0.builder();
        let lhs_value = lhs.build_get_value_max_denom(builder)?;
        let rhs_value = lhs.build_get_value_max_denom(builder)?;
        let new_value = go(lhs_value, rhs_value)?;

        let lhs_log_denom = lhs.build_get_log_denom(builder)?;
        let rhs_log_denom = lhs.build_get_log_denom(builder)?;

        let lhs_log_denom_larger =
            builder.build_int_compare(IntPredicate::UGT, lhs_log_denom, rhs_log_denom, "")?;
        let lhs_larger_r = {
            let v = lhs.build_unmax_denom(builder, new_value)?;
            angle_ty.build_value(builder, v, lhs_log_denom)?
        };
        let rhs_larger_r = {
            let v = rhs.build_unmax_denom(builder, new_value)?;
            angle_ty.build_value(builder, v, rhs_log_denom)?
        };
        let r = builder.build_select(lhs_log_denom_larger, lhs_larger_r, rhs_larger_r, "")?;
        LLVMAngleValue::try_new(angle_ty, r)
    }
}

impl<'c, H: HugrView> EmitOp<'c, ExtensionOp, H> for AngleOpEmitter<'c, '_, H> {
    fn emit(&mut self, args: EmitOpArgs<'c, ExtensionOp, H>) -> Result<()> {
        let module = self.0.get_current_module();
        let float_ty = self.0.iw_context().f64_type();
        let i32_ty = self.0.iw_context().i32_type();
        let builder = self.0.builder();
        let angle_ty = self.1;

        match AngleOp::from_op(&args.node())? {
            AngleOp::atrunc => {
                let [angle, new_log_denom] = args
                    .inputs
                    .try_into()
                    .map_err(|_| anyhow!("AngleOp::atrunc expects two arguments"))?;
                let new_log_denom = new_log_denom.into_int_value();
                let angle = LLVMAngleValue::try_new(angle_ty, angle)?;
                let (value, old_log_denom) = (
                    angle.build_get_value(builder)?,
                    angle.build_get_log_denom(builder)?,
                );

                let denom_increasing = builder.build_int_compare(
                    inkwell::IntPredicate::UGT,
                    new_log_denom,
                    old_log_denom,
                    "",
                )?;

                let increasing_new_value = {
                    let denom_increase = builder.build_int_sub(new_log_denom, old_log_denom, "")?;
                    builder.build_left_shift(value, denom_increase, "")?
                };

                let decreasing_new_value = {
                    let denom_decrease = builder.build_int_sub(old_log_denom, new_log_denom, "")?;
                    builder.build_right_shift(value, denom_decrease, false, "")?
                };

                let value = builder
                    .build_select(
                        denom_increasing,
                        increasing_new_value,
                        decreasing_new_value,
                        "",
                    )?
                    .into_int_value();
                let r = angle_ty.build_value(builder, value, new_log_denom)?;

                args.outputs.finish(builder, [r.into()])
            }
            AngleOp::aadd => {
                let [lhs, rhs] = args
                    .inputs
                    .try_into()
                    .map_err(|_| anyhow!("AngleOp::aadd expects two arguments"))?;
                let r = self.binary_angle_op(
                    LLVMAngleValue::try_new(angle_ty, lhs)?,
                    LLVMAngleValue::try_new(angle_ty, rhs)?,
                    |lhs, rhs| builder.build_int_add(lhs, rhs, ""),
                )?;
                args.outputs.finish(builder, [r.into()])
            }
            AngleOp::asub => {
                let [lhs, rhs] = args
                    .inputs
                    .try_into()
                    .map_err(|_| anyhow!("AngleOp::asub expects one arguments"))?;
                let r = self.binary_angle_op(
                    LLVMAngleValue::try_new(angle_ty, lhs)?,
                    LLVMAngleValue::try_new(angle_ty, rhs)?,
                    |lhs, rhs| builder.build_int_sub(lhs, rhs, ""),
                )?;
                args.outputs.finish(builder, [r.into()])
            }
            AngleOp::aneg => {
                let [angle] = args
                    .inputs
                    .try_into()
                    .map_err(|_| anyhow!("AngleOp::aparts expects one arguments"))?;
                let angle = LLVMAngleValue::try_new(angle_ty, angle)?;
                let log_denom = angle.build_get_log_denom(builder)?;
                let value = {
                    let v = angle.build_get_value_max_denom(builder)?;
                    let v = builder.build_int_neg(v, "")?;
                    angle.build_unmax_denom(builder, v)?
                };
                let r = angle_ty.build_value(builder, value, log_denom)?;
                args.outputs.finish(builder, [r.into()])
            }
            AngleOp::anew => {
                let [value, log_denom] = args
                    .inputs
                    .try_into()
                    .map_err(|_| anyhow!("AngleOp::anew expects two arguments"))?;
                let r = self.1.build_value(builder, value, log_denom)?;
                args.outputs.finish(builder, [r.into()])
            }
            AngleOp::aparts => {
                let [angle] = args
                    .inputs
                    .try_into()
                    .map_err(|_| anyhow!("AngleOp::aparts expects one argument"))?;
                let angle = LLVMAngleValue::try_new(self.1, angle)?;
                let value = angle.build_get_value(builder)?;
                let log_denom = angle.build_get_log_denom(builder)?;
                args.outputs
                    .finish(builder, [value.into(), log_denom.into()])
            }
            AngleOp::afromrad => {
                let [log_denom, rads] = args
                    .inputs
                    .try_into()
                    .map_err(|_| anyhow!("AngleOp::afromrad expects two arguments"))?;
                let log_denom = log_denom.into_int_value();
                let rads: FloatValue<'c> = rads
                    .try_into()
                    .map_err(|_| anyhow!("afromrad expects a float argument"))?;
                let float_ty = rads.get_type();
                let two_pi = float_ty.const_float(PI * 2.0);
                let normalised_rads = {
                    let normalised_rads = {
                        let rads_ok = {
                            let is_fpclass = {
                                let intrinsic = Intrinsic::find("llvm.is.fpclass")
                                    .ok_or(anyhow!("failed to find 'llvm.is.fpclass' intrinsic"))?;
                                intrinsic.get_declaration(module, &[float_ty.as_basic_type_enum(), i32_ty.as_basic_type_enum()])
                                    .ok_or(anyhow!("failed to get_delcaration 'llvm.is.fpclass' intrinsic for {float_ty}"))?
                            };
                            // bit 0: Signalling Nan
                            // bit 3: Negative normal
                            // bit 8: Positive normal
                            let test = i32_ty.const_int((1 << 0) | (1 << 3) | (1 << 8), false);
                            builder
                                .build_call(is_fpclass, &[rads.into(), test.into()], "")?
                                .try_as_basic_value()
                                .left()
                                .ok_or(anyhow!("llvm.is.fpclass has no return value"))?
                                .into_int_value()
                        };
                        let zero = float_ty.const_zero();
                        let ok_rads = builder.build_float_rem(rads, two_pi, "")?;
                        builder
                            .build_select(rads_ok, ok_rads, zero, "")?
                            .into_float_value()
                    };
                    let is_negative = builder.build_float_compare(
                        FloatPredicate::OLT,
                        normalised_rads,
                        rads.get_type().const_zero(),
                        "",
                    )?;
                    let is_negative_r = builder.build_float_add(two_pi, normalised_rads, "")?;
                    let is_positive_r = normalised_rads;
                    builder
                        .build_select(is_negative, is_negative_r, is_positive_r, "")?
                        .into_float_value()
                };
                let value = {
                    let denom = {
                        let log_denom =
                            builder.build_unsigned_int_to_float(log_denom, float_ty, "")?;
                        let exp2 = {
                            let intrinsic = Intrinsic::find("llvm.exp2")
                                .ok_or(anyhow!("failed to find 'llvm.exp2' intrinsic"))?;
                            intrinsic
                                .get_declaration(module, &[float_ty.as_basic_type_enum()])
                                .ok_or(anyhow!(
                                    "failed to get_delcaration 'llvm.exp2' intrinsic for {float_ty}"
                                ))?
                        };
                        builder
                            .build_call(exp2, &[log_denom.into()], "")?
                            .try_as_basic_value()
                            .left()
                            .ok_or(anyhow!("exp2 intrinsic had no return value"))?
                            .into_float_value()
                    };
                    let value = builder.build_float_mul(normalised_rads, denom, "")?;
                    builder.build_float_to_unsigned_int(value, angle_ty.value_field_type(), "")?
                };
                args.outputs.finish(
                    builder,
                    [angle_ty.build_value(builder, value, log_denom)?.into()],
                )
            }
            AngleOp::atorad => {
                let [angle] = args
                    .inputs
                    .try_into()
                    .map_err(|_| anyhow!("AngleOp::atorad expects one arguments"))?;
                let angle = LLVMAngleValue::try_new(angle_ty, angle)?;
                let value = angle.build_get_value(builder)?;
                let log_denom = angle.build_get_log_denom(builder)?;
                let r = {
                    let value = builder.build_unsigned_int_to_float(value, float_ty, "")?;
                    let denom = {
                        let log_denom =
                            builder.build_unsigned_int_to_float(log_denom, float_ty, "")?;
                        let exp2 = {
                            let intrinsic = Intrinsic::find("exp2")
                                .ok_or(anyhow!("failed to find 'exp2' intrinsic"))?;
                            intrinsic
                                .get_declaration(module, &[float_ty.as_basic_type_enum()])
                                .ok_or(anyhow!(
                                    "failed to get_delcaration 'exp2' intrinsic for {float_ty}"
                                ))?
                        };
                        builder
                            .build_call(exp2, &[log_denom.into()], "")?
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
                let (lhs, rhs) = (
                    LLVMAngleValue::try_new(angle_ty, lhs)?,
                    LLVMAngleValue::try_new(angle_ty, rhs)?,
                );
                let lhs_value = lhs.build_get_value_max_denom(builder)?;
                let rhs_value = rhs.build_get_value_max_denom(builder)?;
                let r = {
                    let r_i1 =
                        builder.build_int_compare(IntPredicate::EQ, lhs_value, rhs_value, "")?;
                    let true_val = emit_value(self.0, &Value::true_val())?;
                    let false_val = emit_value(self.0, &Value::false_val())?;
                    self.0
                        .builder()
                        .build_select(r_i1, true_val, false_val, "")?
                };
                args.outputs.finish(self.0.builder(), [r.into()])
            }
            AngleOp::amul => {
                let [lhs, rhs] = args
                    .inputs
                    .try_into()
                    .map_err(|_| anyhow!("AngleOp::amul expects two arguments"))?;
                let r = self.binary_angle_op(
                    LLVMAngleValue::try_new(angle_ty, lhs)?,
                    LLVMAngleValue::try_new(angle_ty, rhs)?,
                    |lhs, rhs| builder.build_int_mul(lhs, rhs, ""),
                )?;
                args.outputs.finish(builder, [r.into()])
            }
            AngleOp::adiv => {
                let [lhs, rhs] = args
                    .inputs
                    .try_into()
                    .map_err(|_| anyhow!("AngleOp::adiv expects two arguments"))?;
                let r = self.binary_angle_op(
                    LLVMAngleValue::try_new(angle_ty, lhs)?,
                    LLVMAngleValue::try_new(angle_ty, rhs)?,
                    |lhs, rhs| builder.build_int_mul(lhs, rhs, ""),
                )?;
                args.outputs.finish(builder, [r.into()])
            }
            _ => todo!(),
        }
    }
}
