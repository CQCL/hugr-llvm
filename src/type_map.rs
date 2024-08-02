use std::{collections::HashMap, rc::Rc};

use anyhow::{anyhow, Result};
use hugr::{
    extension::ExtensionId,
    types::{CustomType, TypeEnum, TypeName, TypeRow},
};
use itertools::Either;

use crate::types::{HugrSumType, HugrType};

pub mod def_hook;
pub mod destructor;

pub trait TypeMapping<'a, TM: TypeMappable<'a>>: Fn(TM::InV) -> Result<TM::OutV> {}
impl<'a, TM: TypeMappable<'a>, F: Fn(TM::InV) -> Result<TM::OutV> + ?Sized> TypeMapping<'a, TM>
    for F
{
}

pub trait TypeMappable<'a> {
    type InV: Clone;
    type OutV;
    fn aggregate_variants(
        sum_type: &HugrSumType,
        inv: Self::InV,
        variants: impl IntoIterator<Item = Vec<Option<Self::OutV>>>,
    ) -> Option<Self::OutV>;
    // fn disaggregate_variants(sum_type: &HugrSumType, v: &Self::InV) -> impl Iterator<Item=Vec<Self::InV>>;
}

pub type CustomTypeKey = (ExtensionId, TypeName);

pub struct TypeMap<'a, TM: TypeMappable<'a>> {
    // pub typing_session: Rc<TypingSession<'c, H>>,,
    custom_hooks: HashMap<CustomTypeKey, Rc<dyn TypeMapping<'a, TM> + 'a>>,
    // marker: std::marker::PhantomData<&'s ()>
}

impl<'a, TM: TypeMappable<'a>> Clone for TypeMap<'a, TM> {
    fn clone(&self) -> Self {
        Self {
            custom_hooks: self.custom_hooks.clone(),
        }
    }
}

impl<'a, TM: TypeMappable<'a>> Default for TypeMap<'a, TM> {
    fn default() -> Self {
        Self {
            custom_hooks: Default::default(),
        }
    }
}

fn map_either<'a, TM: TypeMappable<'a>>(
    e: Either<impl TypeMapping<'a, TM>, impl TypeMapping<'a, TM>>,
    inv: TM::InV,
) -> Result<TM::OutV> {
    match e {
        Either::Left(ref hook) => hook(inv),
        Either::Right(ref hook) => hook(inv),
    }
}

impl<'a, TM: TypeMappable<'a>> TypeMap<'a, TM> {
    pub fn set_leaf_hook(&mut self, hugr_type: CustomTypeKey, hook: impl TypeMapping<'a, TM> + 'a) {
        self.custom_hooks.insert(hugr_type, Rc::new(hook));
    }

    pub fn map(&self, hugr_type: &HugrType, inv: TM::InV) -> Result<Option<TM::OutV>> {
        match hugr_type.as_type_enum() {
            TypeEnum::Extension(custom_type) => self.custom_type(custom_type, inv),
            TypeEnum::Sum(sum_type) => self.sum_type(sum_type, inv),
            _ => Err(anyhow!("unsupported type: {hugr_type}")),
        }
    }

    fn custom_type(&self, custom_type: &CustomType, inv: TM::InV) -> Result<Option<TM::OutV>> {
        let key = (custom_type.extension().clone(), custom_type.name().clone());
        let Some(hook) = self.custom_hooks.get(&key) else {
            return Ok(None);
        };

        hook(inv).map(Some)
    }

    fn sum_type(&self, sum_type: &HugrSumType, inv: TM::InV) -> Result<Option<TM::OutV>> {
        let inv2 = inv.clone();
        Ok(TM::aggregate_variants(
            sum_type,
            inv,
            (0..sum_type.num_variants())
                .map(move |i| {
                    let tr: TypeRow = sum_type.get_variant(i).unwrap().clone().try_into().unwrap();
                    tr.iter()
                        .map(|t| self.map(t, inv2.clone()))
                        .collect::<Result<Vec<_>>>()
                })
                .collect::<Result<Vec<_>>>()?,
        ))
    }

    // pub fn add_composite_hook(&mut self, hugr_type: CustomType, components: HugrType) {}
}
