use anyhow::{anyhow, Result};
use std::{collections::HashMap, marker::PhantomData, rc::Rc};

use hugr::{types::CustomType, HugrView, Wire};
use inkwell::{builder::Builder, context::AsContextRef, values::BasicValue};
use itertools::{zip_eq, Itertools};

use crate::{
    emit::func::MailBoxDefHook,
    sum::{LLVMSumType, LLVMSumValue},
    types::{HugrSumType, HugrType, TypingSession},
};

use super::{CustomTypeKey, TypeMap, TypeMappable, TypeMapping};

pub struct DefHookTypeMap<'a, 'c, H>(TypeMap<'a, DefHookTypeMapping<'a, 'c, H>>);

impl<'a, 'c, H> Default for DefHookTypeMap<'a, 'c, H> {
    fn default() -> Self {
        Self(Default::default())
    }
}

impl<'a, 'c, H> Clone for DefHookTypeMap<'a, 'c, H> {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl<'a, 'c, H> DefHookTypeMap<'a, 'c, H> {
    pub fn new(_: impl AsContextRef<'c>, _: &'a H) -> Self {
        Self::default()
    }

    pub fn get_def_hook(
        &self,
        hugr_type: &HugrType,
        typing_session: TypingSession<'c, H>,
        hugr: &'a H,
        wire: Wire,
    ) -> Result<Option<Rc<dyn MailBoxDefHook<'c> + 'a>>> {
        self.0
            .map(hugr_type, DefHookInV(typing_session, hugr, wire))
    }

    pub fn set_def_hook(
        &mut self,
        custom_type: CustomTypeKey,
        hook: impl TypeMapping<'a, DefHookTypeMapping<'a, 'c, H>> + 'a,
    ) {
        self.0.set_leaf_hook(custom_type, Box::new(hook))
    }
}

#[derive(Default)]
pub struct DefHookTypeMapping<'a, 'c, H>(PhantomData<&'a &'c H>);

pub struct DefHookInV<'a, 'c, H>(pub TypingSession<'c, H>, pub &'a H, pub Wire);

impl<'a, 'c, H> Clone for DefHookInV<'a, 'c, H> {
    fn clone(&self) -> Self {
        Self(self.0.clone(), self.1.clone(), self.2.clone())
    }
}

// impl<'a,'c,H> DefHookInV<'a, 'c, H> {
//     pub fn new(typing_session: TypingSession<'c,H>, hugr: &'a H, wire: Wire) -> Self {
//         Self(typing_session,hugr,wire)
//     }

// }

impl<'a, 'c, H> TypeMappable<'a> for DefHookTypeMapping<'a, 'c, H> {
    type InV = DefHookInV<'a, 'c, H>;
    type OutV = Rc<dyn MailBoxDefHook<'c> + 'a>;

    fn noop(_: &Self::InV) -> Self::OutV {
        Rc::new(move |_, value| Ok(value))
    }

    fn aggregate_variants(
        sum_type: &HugrSumType,
        inv: Self::InV,
        variants: impl IntoIterator<Item = Vec<Option<Self::OutV>>>,
    ) -> Option<Self::OutV> {
        let variant_hooks = variants.into_iter().collect::<Vec<_>>();
        let sum_type = Rc::new(LLVMSumType::try_new(&inv.0, sum_type.clone()).unwrap());

        if variant_hooks.iter().all(|v| v.iter().all(Option::is_none)) {
            return None;
        }

        Some(Rc::new(move |builder: &Builder<'c>, value| {
            let sum_value = LLVMSumValue::try_new(value, sum_type.as_ref().clone())?;
            let current_block = builder
                .get_insert_block()
                .ok_or(anyhow!("no current block"))?;
            let exit_block = current_block
                .get_context()
                .insert_basic_block_after(current_block, "");
            let mut incomings = vec![];
            sum_value.build_destructure(builder, |builder, tag, mut vs| {
                let hooks = &variant_hooks[tag];
                if hooks.iter().any(|x| !x.is_none()) {
                    vs = zip_eq(hooks, vs)
                        .map(|(hook, v)| {
                            if let Some(hook) = hook {
                                hook(builder, v)
                            } else {
                                Ok(v)
                            }
                        })
                        .collect::<Result<Vec<_>>>()?;
                }
                let v = sum_type.build_tag(builder, tag, vs)?;
                incomings.push((Box::new(v), builder.get_insert_block().unwrap()));
                builder.build_unconditional_branch(exit_block)?;
                Ok(())
            })?;
            builder.position_at_end(exit_block);
            let value = builder.build_phi(value.get_type(), "")?;
            value.add_incoming(
                incomings
                    .iter()
                    .map(|(v, b)| (v.as_ref() as &dyn BasicValue<'c>, *b))
                    .collect_vec()
                    .as_slice(),
            );
            Ok(value.as_basic_value())
        }))
    }

    // fn disaggregate_variants(sum_type: &crate::types::HugrSumType, v: &Self::InV) -> impl Iterator<Item=Vec<Self::InV>> {
    //     todo!()
    // }
}

#[cfg(test)]
mod test {
    use hugr::{
        builder::{DFGBuilder, Dataflow, DataflowHugr, ModuleBuilder},
        std_extensions::ptr::{self, ptr_custom_type, ptr_type, PTR_REG},
        types::{Signature, Type},
        HugrView,
    };
    use inkwell::{
        builder::Builder,
        types::BasicType,
        values::{BasicValue, CallableValue, PointerValue},
    };
    use rstest::rstest;

    use crate::{
        test::{llvm_ctx, TestContext},
        type_map::def_hook::DefHookTypeMap,
    };

    use super::*;

    #[rstest]
    fn ptr_hook(llvm_ctx: TestContext) {
        let ptr_custom_type = ptr_custom_type(Type::UNIT);
        let hugr = {
            let builder =
                DFGBuilder::new(Signature::new_endo(vec![ptr_custom_type.clone().into(); 3]))
                    .unwrap();
            let [_, i2, i3] = builder.input_wires_arr();
            builder
                .finish_hugr_with_outputs([i2, i3, i3], &PTR_REG)
                .unwrap()
        };
        let mut def_hooks = DefHookTypeMap::new(llvm_ctx.iw_context(), &hugr);

        fn mk_def_hook<'a, 'c, H: HugrView>(
            hugr: &'a H,
            wire: Wire,
        ) -> impl MailBoxDefHook<'c> + 'a {
            move |builder: &Builder, value| {
                let context = builder.get_insert_block().unwrap().get_context();
                let ref_count_type = context.i64_type();
                let fn_type = context.void_type().fn_type(
                    &[
                        value
                            .get_type()
                            .ptr_type(Default::default())
                            .as_basic_type_enum()
                            .into(),
                        ref_count_type.as_basic_type_enum().into(),
                    ],
                    false,
                );
                let num_uses = hugr.linked_inputs(wire.node(), wire.source()).count();
                match num_uses {
                    0 => {
                        builder.build_call(
                            CallableValue::try_from(
                                fn_type.ptr_type(Default::default()).get_undef(),
                            )
                            .unwrap(),
                            &[value.into(), ref_count_type.const_int(1, false).into()],
                            "",
                        )?;
                    }
                    x if x > 1 => {
                        builder.build_call(
                            CallableValue::try_from(
                                fn_type.ptr_type(Default::default()).get_undef(),
                            )
                            .unwrap(),
                            &[
                                value.into(),
                                ref_count_type.const_int((x - 1) as u64, false).into(),
                            ],
                            "",
                        )?;
                    }
                    _ => (),
                };
                Ok(value)
            }
        }

        def_hooks.set_def_hook(
            (
                ptr_custom_type.extension().clone(),
                ptr_custom_type.name().clone(),
            ),
            move |DefHookInV(_, hugr, wire)| Ok(Rc::new(mk_def_hook(hugr, wire))),
        );
    }
}
