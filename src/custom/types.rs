use std::marker::PhantomData;

use itertools::Itertools as _;

use hugr::types::CustomType;

use anyhow::Result;
use inkwell::types::{BasicMetadataTypeEnum, BasicType as _, BasicTypeEnum, FunctionType};

pub use crate::utils::type_map::CustomTypeKey;

use crate::{
    sum::LLVMSumType,
    types::{HugrFuncType, HugrSumType, TypingSession},
    utils::type_map::{TypeMapping, TypeMappingFn},
};

// pub trait LLVMCustomTypeFn<'a>;
// :
//     for<'c> Fn(TypingSession<'c, 'a>, &CustomType) -> Result<BasicTypeEnum<'c>> + 'a
// {

// }

// impl<
//         'tm,
//         F: for<'c> Fn(TypingSession<'c, 'tm>, &CustomType) -> Result<BasicTypeEnum<'c>> + 'tm + ?Sized,
//     > TypeMappingFn<'tm, LLVMTypeMapping<'tm>> for F
// {
//     fn map_type<'c>(&self, inv: <LLVMTypeMapping<'tm> as TypeMapping<'tm>>::InV<'c>, ty: &CustomType) -> Result<<LLVMTypeMapping<'tm> as TypeMapping<'tm>>::OutV<'c>> where 'tm: 'c {
//         self(inv,ty)
//     }
// }

#[derive(Default, Clone)]
pub struct LLVMTypeMapping<'a>(PhantomData<&'a ()>);

impl<'tm> TypeMapping<'tm> for LLVMTypeMapping<'tm>  {
    type InV<'c> = TypingSession<'c, 'tm> where 'tm: 'c
    // where Self: 'c
    ;

    type OutV<'c> = BasicTypeEnum<'c> where 'tm: 'c
        // Self: 'c
        ;

    type SumOutV<'c> = LLVMSumType<'c> where 'tm: 'c
        // Self: 'c
        ;

    type FuncOutV<'c> = FunctionType<'c> where 'tm: 'c
        // Self: 'c
        ;

    fn sum_into_out<'c>(&self, sum: Self::SumOutV<'c>) -> Self::OutV<'c> where 'tm: 'c {
        sum.as_basic_type_enum()
    }

    fn func_into_out<'c>(&self, sum: Self::FuncOutV<'c>) -> Self::OutV<'c> where 'tm: 'c{
        sum.ptr_type(Default::default()).as_basic_type_enum()
    }

    fn map_sum_type<'c>(
        &self,
        sum_type: &HugrSumType,
        context: TypingSession<'c, 'tm>,
        variants: impl IntoIterator<Item = Vec<Self::OutV<'c>>>,
    ) -> Result<Self::SumOutV<'c>> where 'tm: 'c{
        LLVMSumType::try_new2(
            context.iw_context(),
            variants.into_iter().collect(),
            sum_type.clone(),
        )
    }

    fn map_function_type<'c>(
        &self,
        _: &HugrFuncType,
        context: TypingSession<'c, 'tm>,
        inputs: impl IntoIterator<Item = Self::OutV<'c>>,
        outputs: impl IntoIterator<Item = Self::OutV<'c>>,
    ) -> Result<Self::FuncOutV<'c>> where 'tm: 'c{
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
