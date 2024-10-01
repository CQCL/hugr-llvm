use itertools::Itertools as _;

use hugr::types::CustomType;

use anyhow::{bail, Result};
use inkwell::types::{BasicMetadataTypeEnum, BasicType as _, BasicTypeEnum, FunctionType};

use crate::{
    sum::LLVMSumType,
    types::{HugrFuncType, HugrSumType, HugrType, TypingSession},
    utils::type_map::TypeMapping,
};

pub trait LLVMCustomTypeFn<'a>:
    for<'c> Fn(TypingSession<'c>, &CustomType) -> Result<BasicTypeEnum<'c>> + 'a
{
}

impl<
        'a,
        F: for<'c> Fn(TypingSession<'c>, &CustomType) -> Result<BasicTypeEnum<'c>> + 'a + ?Sized,
    > LLVMCustomTypeFn<'a> for F
{
}

#[derive(Default, Clone)]
pub struct LLVMTypeMapping;

impl TypeMapping for LLVMTypeMapping {
    type InV<'c> = TypingSession<'c>;

    type OutV<'c> = BasicTypeEnum<'c>;

    type SumOutV<'c> = LLVMSumType<'c>;

    type FuncOutV<'c> = FunctionType<'c>;

    fn sum_into_out<'c>(&self, sum: Self::SumOutV<'c>) -> Self::OutV<'c> {
        sum.as_basic_type_enum()
    }

    fn func_into_out<'c>(&self, sum: Self::FuncOutV<'c>) -> Self::OutV<'c> {
        sum.ptr_type(Default::default()).as_basic_type_enum()
    }

    fn default_out<'c>(&self, hugr_type: &HugrType) -> Result<Self::OutV<'c>> {
        bail!("Unsupported type: {hugr_type}")
    }

    fn map_sum_type<'c>(
        &self,
        sum_type: &HugrSumType,
        context: TypingSession<'c>,
        variants: impl IntoIterator<Item = Vec<Self::OutV<'c>>>,
    ) -> Result<Self::SumOutV<'c>> {
        LLVMSumType::try_new2(
            context.iw_context(),
            variants.into_iter().collect(),
            sum_type.clone(),
        )
    }

    fn map_function_type<'c>(
        &self,
        _: &HugrFuncType,
        context: TypingSession<'c>,
        inputs: impl IntoIterator<Item = Self::OutV<'c>>,
        outputs: impl IntoIterator<Item = Self::OutV<'c>>,
    ) -> Result<Self::FuncOutV<'c>> {
        let iw_context = context.iw_context();
        let inputs: Vec<BasicMetadataTypeEnum<'c>> = inputs.into_iter().map_into().collect_vec();
        let outputs = outputs.into_iter().collect_vec();
        Ok(match outputs.as_slice() {
            &[] => iw_context.void_type().fn_type(&inputs, false),
            [res] => res.fn_type(&inputs, false),
            ress => iw_context.struct_type(ress, false).fn_type(&inputs, false),
        })
    }
}
