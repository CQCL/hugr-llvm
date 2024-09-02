use hugr::extension::ExtensionId;
use hugr::types::{CustomType};
use llvm_sys_140::LLVMValue;

use crate::emit::{
    func::EmitFuncContext, ops::emit_custom_unary_op,
    EmitOp, EmitOpArgs,
};

use hugr::std_extensions::arithmetic::conversions::ConvertOpDef;

use super::{CodegenExtension, CodegenExtsMap};
use crate::types::TypingSession;
use anyhow::{anyhow, Result};

use hugr::{HugrView, std_extensions::arithmetic::{conversions, int_types::INT_TYPES}, ops::CustomOp};

use hugr::extension::simple_op::MakeExtensionOp;
use hugr::types::SumType;
use hugr::extension::prelude::ERROR_TYPE;

use inkwell::types::BasicTypeEnum;
use inkwell::values::{AnyValue, BasicValue, BasicValueEnum};

use crate::emit::ops::emit_value;

use hugr::ops::constant::Value;

use inkwell::builder::Builder;

use crate::sum::LLVMSumType;

struct ConversionsEmitter<'c, 'd, H>(&'d mut EmitFuncContext<'c, H>);

/*
fn emit_trunc_op<'c>(builder: Builder<'c>, int_type: BasicTypeEnum, float_value: BasicValueEnum) -> Result<()> {
    // Make a maximum size int and convert it to float to work out if the
    // truncation should succeed.
    let x = builder.build_float_to_unsigned_int(float_value.into_float_value(), int_type.into_int_type(), "")?;
    // If x is poison, we need to return a failure tag
    if x.is_poison() {
        emit_value(builder., &Value::false_val())?;
    }
    // Otherwise, we need to pack the result in a success tag
    else {
        todo!()
    }

    Ok(())
}
 */


fn truncate_output(width: u8) -> SumType {
    SumType::new([
        INT_TYPES[width as usize].clone(),
        ERROR_TYPE,
    ])
}


impl<'c, H: HugrView> EmitOp<'c, CustomOp, H> for ConversionsEmitter<'c, '_, H> {
    fn emit(&mut self, args: EmitOpArgs<'c, CustomOp, H>) -> Result<()> {
        let conversion_op = ConvertOpDef::from_optype(&args.node().generalise()).ok_or(anyhow!(
            "ConversionsEmitter from_optype failed: {:?}",
            args.node().as_ref()
        ))?;
        match conversion_op {
            ConvertOpDef::trunc_u => {
                // TODO: Be generic over the width - work it out from what's on the hugr node
                let sum_ty = LLVMSumType::try_new(&self.0.typing_session(), truncate_output(5))?;
                emit_custom_unary_op(self.0, args, |builder, arg, out_tys| {
                    // TODO proper error handling
                    let [out_ty] = out_tys else { panic!(""); };
                    let int_ty = out_ty.into_struct_type().get_field_type_at_index(0).unwrap().into_int_type();
                    let trunc_result = builder
                        .build_float_to_unsigned_int(arg.into_float_value(), int_ty, "")?
                        .as_basic_value_enum();
                    let optional_result = if trunc_result.is_poison() {
                        // TODO: Construct an error object
                        sum_ty.build_tag(builder, 1, vec![])
                    } else {
                        sum_ty.build_tag(builder, 0, vec![trunc_result])
                    }?;

                    Ok(vec![optional_result])
                })
            },

            ConvertOpDef::trunc_s => {
                let sum_ty = LLVMSumType::try_new(&self.0.typing_session(), truncate_output(5))?;
                emit_custom_unary_op(self.0, args, |builder, arg, out_tys| {
                    // TODO proper error handling
                    let [out_ty] = out_tys else { panic!(""); };
                    let int_ty = out_ty.into_struct_type().get_field_type_at_index(0).unwrap().into_int_type();
                    let trunc_result = builder
                        .build_float_to_signed_int(arg.into_float_value(), int_ty, "")?
                        .as_basic_value_enum();
                    let optional_result = if trunc_result.is_poison() {
                        // TODO: Construct an error object
                        sum_ty.build_tag(builder, 1, vec![])
                    } else {
                        sum_ty.build_tag(builder, 0, vec![trunc_result])
                    }?;

                    Ok(vec![optional_result])
                })
            },

            ConvertOpDef::convert_u => emit_custom_unary_op(self.0, args, |builder, arg, out_tys| {
                    // TODO proper error handling
                let [out_ty] = out_tys else {  panic!(""); };
                Ok(vec![builder
                    .build_unsigned_int_to_float(arg.into_int_value(), out_ty.into_float_type(), "")?
                    .as_basic_value_enum()])
            }),
            ConvertOpDef::convert_s => emit_custom_unary_op(self.0, args, |builder, arg, out_tys| {
                    // TODO proper error handling
                let [out_ty] = out_tys else { panic!(""); };
                Ok(vec![builder
                    .build_signed_int_to_float(arg.into_int_value(), out_ty.into_float_type(), "")?
                    .as_basic_value_enum()])
            }),
            _ => Err(anyhow!("Conversion op not implemented: {:?}", args.node().as_ref())),
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
    ) -> Box<dyn EmitOp<'c, CustomOp, H> + 'a> {
        Box::new(ConversionsEmitter(context))
    }


}


pub fn add_conversions_extension<H: HugrView>(cem: CodegenExtsMap<'_, H>) -> CodegenExtsMap<'_, H> {
    cem.add_cge(ConversionsCodegenExtension)
}

#[cfg(test)]
mod test {

    use super::*;

    use hugr::extension::prelude::ERROR_TYPE_NAME;
    use rstest::rstest;
    use crate::check_emission;
    use crate::test::{llvm_ctx, TestContext};
    use hugr::{
        builder::{Dataflow, DataflowSubContainer},
        extension::prelude::ERROR_TYPE,
        std_extensions::arithmetic::{
            conversions::{CONVERT_OPS_REGISTRY, EXTENSION},
            float_types::FLOAT64_TYPE,
            int_types::INT_TYPES,
        },
        type_row,
        types::{SumType, Type},
        Hugr,
    };
    use crate::emit::test::SimpleHugrConfig;
    use crate::custom::{float::add_float_extensions, int::add_int_extensions, prelude::add_default_prelude_extensions};

    fn test_conversion_op(name: impl AsRef<str>, in_type: Type, out_type: Type, int_width: u8) -> Hugr {
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
                        &CONVERT_OPS_REGISTRY
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
    fn test_convert(mut llvm_ctx: TestContext) -> () {
        let op_name = "convert_u";
        let width = 5;
        llvm_ctx.add_extensions(add_int_extensions);
        llvm_ctx.add_extensions(add_float_extensions);
        llvm_ctx.add_extensions(add_conversions_extension);
        let in_ty = INT_TYPES[width as usize].clone();
        let out_ty = FLOAT64_TYPE;
        let hugr = test_conversion_op(op_name.clone(), in_ty, out_ty, width);
        check_emission!(op_name, hugr, llvm_ctx);
    }

    #[rstest]
    #[case::trunc("trunc_u", 23.4, 5)]
    fn test_truncation(mut llvm_ctx: TestContext, #[case] op_name: &str, #[case] input: f64, #[case] width: u8) -> () {
        llvm_ctx.add_extensions(add_int_extensions);
        llvm_ctx.add_extensions(add_float_extensions);
        llvm_ctx.add_extensions(add_conversions_extension);
        llvm_ctx.add_extensions(add_default_prelude_extensions);
        let in_ty = FLOAT64_TYPE;
        let out_ty = SumType::new([
            INT_TYPES[width as usize].clone(),
            ERROR_TYPE,
        ]);
        let hugr = test_conversion_op(op_name.clone(), in_ty, out_ty.into(), width);
        check_emission!(op_name, hugr, llvm_ctx);
    }
}
