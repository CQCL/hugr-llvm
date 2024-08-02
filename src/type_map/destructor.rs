use anyhow::{anyhow, Result};
use itertools::{zip_eq, Itertools as _};
use std::marker::PhantomData;

use inkwell::{
    builder::Builder, context::AsContextRef, module::Module, types::BasicType, values::{FunctionValue, PointerValue}
};

use crate::{
    sum::{LLVMSumType, LLVMSumValue},
    types::{HugrSumType, HugrType, TypingSession},
};

use super::{CustomTypeKey, TypeMap, TypeMappable, TypeMapping};

pub struct Destructor<'a, 'c, H>(TypeMap<'a, DestructorTypeMapping<'a, 'c, H>>);

#[derive(Default)]
pub struct DestructorTypeMapping<'a, 'c, H>(PhantomData<&'a &'c H>);

pub struct DestructorInV<'a, 'c, H>(pub TypingSession<'c, H>, pub &'a H);

impl<'a, 'c, H> Clone for DestructorInV<'a, 'c, H> {
    fn clone(&self) -> Self {
        Self(self.0.clone(), self.1)
    }
}

pub trait BuildDestructor<'c>: Fn(&Builder<'c>, PointerValue<'c>) -> Result<()> {}
impl<'c, F: Fn(&Builder<'c>, PointerValue<'c>) -> Result<()> + ?Sized> BuildDestructor<'c> for F {}

impl<'a, 'c, H> TypeMappable<'a> for DestructorTypeMapping<'a, 'c, H> {
    type InV = (&'a Module<'c>, TypingSession<'c, H>);
    type OutV = Result<FunctionValue<'c>>;

    fn aggregate_variants(
        sum_type: &HugrSumType,
        (module, session): (&Module<'c>, TypingSession<'c, H>),
        variants: impl IntoIterator<Item = Vec<Option<Self::OutV>>>,
    ) -> Option<Result<FunctionValue<'c>>> {
        let name = format!("_prefix.destructor_{sum_type}");
        if let Some(func) = module.get_function(&name) {
            return Some(Ok(func));
        }

        let variant_destructors = match (move || {
            let transpose_variants = variants
                .into_iter()
                .map(|row_ds| {
                    let transpose_rows = row_ds
                        .into_iter()
                        .map(|mb_d| mb_d.map_or(Ok(None), |d| Ok(Some(d?))))
                        .collect::<Result<Vec<_>>>()?;
                    Ok(transpose_rows)
                })
                .collect::<Result<Vec<_>>>()?;
            Ok(transpose_variants)
        })() {
            Err(e) => return Some(Err(e)),
            Ok(x) => x,
        };

        if variant_destructors
            .iter()
            .all(|v| v.iter().all(|x| x.is_none()))
        {
            return None;
        };

        Some((move || {
            let sum_type = LLVMSumType::try_new(&session, sum_type.clone())?;
            let ctx = module.get_context();
            let destructor_type = ctx
                .void_type()
                .fn_type(&[ctx.i8_type().ptr_type(Default::default()).into()], false);
            let func = module.add_function(&name, destructor_type, None);
            let entry_block = ctx.append_basic_block(func, "entry");
            let builder = module.get_context().create_builder();
            builder.position_at_end(entry_block);
            let ptr = {
                let mut ptr = func.get_first_param().unwrap().into_pointer_value();
                let val_ptr_type = sum_type.ptr_type(Default::default());
                if ptr.get_type() != val_ptr_type {
                    ptr = builder
                        .build_bitcast(ptr, val_ptr_type, "")?
                        .into_pointer_value();
                }
                ptr
            };

            let val = builder.build_load(ptr, "val")?;
            let val = LLVMSumValue::try_new(val, sum_type)?;
            val.build_destructure(&builder, move |builder, tag, vs| {
                for (mb_destructor, v) in zip_eq(&variant_destructors[tag], vs) {
                    if let Some(destructor) = mb_destructor {
                        builder.build_call(*destructor, &[v.into()], "")?;
                    }
                }
                builder.build_return(None)?;
                Ok(())
            })?;
            Ok(func)
        })())
    }
}

fn get_null_constructor<'c>(module: &Module<'c>) -> Result<FunctionValue<'c>> {
    let name = format!("_prefix.destructor_null");
    if let Some(func) = module.get_function(&name) {
        return Ok(func);
    }

    let ctx = module.get_context();
    let destructor_type = ctx
        .void_type()
        .fn_type(&[ctx.i8_type().ptr_type(Default::default()).into()], false);
    let func = module.add_function(&name, destructor_type, None);
    let bb = ctx.append_basic_block(func, "");
    let builder = ctx.create_builder();
    builder.position_at_end(bb);
    builder.build_return(None)?;
    Ok(func)
}

impl<'a, 'c, H> Destructor<'a, 'c, H> {
    pub fn set_destructor(
        &mut self,
        custom_type: CustomTypeKey,
        build_destructor: impl TypeMapping<'a, DestructorTypeMapping<'a, 'c, H>> + 'a,
    ) {
        self.0
            .set_leaf_hook(custom_type, Box::new(build_destructor))
    }

    pub fn get_destructor(&self, module: &'a Module<'c>, session: TypingSession<'c,H>, hugr_type: &HugrType) -> Result<FunctionValue<'c>> {
        self.0.map(hugr_type, (module, session)).and_then(|mb_destructor| mb_destructor.unwrap_or_else(|| get_null_constructor(module)))
    }
}
