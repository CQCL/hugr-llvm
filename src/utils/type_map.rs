//! Provides a generic mapping from [hugr::Type] to some domain.
use std::collections::HashMap;

use hugr::{
    extension::ExtensionId,
    types::{CustomType, TypeEnum, TypeName, TypeRow},
};

use anyhow::Result;

use crate::types::{HugrFuncType, HugrSumType, HugrType};

pub trait TypeMapFnHelper<'c, TM: TypeMapping>:
    Fn(TM::InV<'c>, &CustomType) -> Result<TM::OutV<'c>>
{
}
impl<'c, TM: TypeMapping, F> TypeMapFnHelper<'c, TM> for F where
    F: Fn(TM::InV<'c>, &CustomType) -> Result<TM::OutV<'c>> + ?Sized
{
}

pub trait TypeMappingFn<'a, TM: TypeMapping>: for<'c> TypeMapFnHelper<'c, TM> + 'a {}
impl<'a, TM: TypeMapping, F: for<'c> TypeMapFnHelper<'c, TM> + ?Sized + 'a> TypeMappingFn<'a, TM>
    for F
{
}

/// Desscribes a
pub trait TypeMapping {
    type InV<'c>: Clone;
    type OutV<'c>;
    type SumOutV<'c>;
    type FuncOutV<'c>;

    fn default_out<'c>(&self, hugr_type: &HugrType) -> Result<Self::OutV<'c>>;

    fn sum_into_out<'c>(&self, sum: Self::SumOutV<'c>) -> Self::OutV<'c>;

    fn func_into_out<'c>(&self, sum: Self::FuncOutV<'c>) -> Self::OutV<'c>;

    fn map_sum_type<'c>(
        &self,
        sum_type: &HugrSumType,
        inv: Self::InV<'c>,
        variants: impl IntoIterator<Item = Vec<Self::OutV<'c>>>,
    ) -> Result<Self::SumOutV<'c>>;

    fn map_function_type<'c>(
        &self,
        function_type: &HugrFuncType,
        inv: Self::InV<'c>,
        inputs: impl IntoIterator<Item = Self::OutV<'c>>,
        outputs: impl IntoIterator<Item = Self::OutV<'c>>,
    ) -> Result<Self::FuncOutV<'c>>; // fn disaggregate_variants(sum_type: &HugrSumType, v: &Self::InV) -> impl Iterator<Item=Vec<Self::InV>>;
}

pub type CustomTypeKey = (ExtensionId, TypeName);

#[derive(Default)]
pub struct TypeMap<'a, TM: TypeMapping> {
    type_map: TM,
    custom_hooks: HashMap<CustomTypeKey, Box<dyn TypeMappingFn<'a, TM>>>,
}

impl<'a, TM: TypeMapping + 'a> TypeMap<'a, TM> {
    pub fn set_callback(
        &mut self,
        hugr_type: CustomTypeKey,
        hook: impl TypeMappingFn<'a, TM> + 'a,
    ) -> bool {
        self.custom_hooks
            .insert(hugr_type, Box::new(hook))
            .is_none()
    }

    pub fn map_type<'c>(&self, hugr_type: &HugrType, inv: TM::InV<'c>) -> Result<TM::OutV<'c>> {
        match hugr_type.as_type_enum() {
            TypeEnum::Extension(custom_type) => self.custom_type(custom_type, inv),
            TypeEnum::Sum(sum_type) => self
                .map_sum_type(sum_type, inv)
                .map(|x| self.type_map.sum_into_out(x)),
            TypeEnum::Function(function_type) => self
                .map_function_type(&function_type.as_ref().clone().try_into()?, inv)
                .map(|x| self.type_map.func_into_out(x)),
            _ => self.type_map.default_out(hugr_type),
        }
    }

    fn custom_type<'c>(&self, custom_type: &CustomType, inv: TM::InV<'c>) -> Result<TM::OutV<'c>> {
        let key = (custom_type.extension().clone(), custom_type.name().clone());
        let Some(handler) = self.custom_hooks.get(&key) else {
            return self.type_map.default_out(&custom_type.clone().into());
        };
        handler(inv, custom_type)
    }

    pub fn map_sum_type<'c>(
        &self,
        sum_type: &HugrSumType,
        inv: TM::InV<'c>,
    ) -> Result<TM::SumOutV<'c>> {
        let inv2 = inv.clone();
        self.type_map.map_sum_type(
            sum_type,
            inv,
            (0..sum_type.num_variants())
                .map(move |i| {
                    let tr: TypeRow = sum_type.get_variant(i).unwrap().clone().try_into().unwrap();
                    tr.iter()
                        .map(|t| self.map_type(t, inv2.clone()))
                        .collect::<Result<Vec<_>>>()
                })
                .collect::<Result<Vec<_>>>()?,
        )
    }

    pub fn map_function_type<'c>(
        &self,
        func_type: &HugrFuncType,
        inv: TM::InV<'c>,
    ) -> Result<TM::FuncOutV<'c>> {
        let inputs = func_type
            .input()
            .iter()
            .map(|t| self.map_type(t, inv.clone()))
            .collect::<Result<Vec<_>>>()?;
        let outputs = func_type
            .output()
            .iter()
            .map(|t| self.map_type(t, inv.clone()))
            .collect::<Result<Vec<_>>>()?;
        self.type_map
            .map_function_type(func_type, inv, inputs, outputs)
    }
}
