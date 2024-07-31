use std::{borrow::Borrow, cell::RefCell, collections::HashMap, rc::Rc};

use anyhow::{anyhow, Result};
use hugr::{
    types::{CustomType, TypeEnum, TypeRow},
    HugrView, Node, OutgoingPort,
};
use inkwell::{
    builder::Builder,
    values::{BasicValue, BasicValueEnum},
};
use itertools::{zip_eq, Either, Itertools};

use crate::{
    emit::func::MailBoxDefHook,
    sum::{LLVMSumType, LLVMSumValue},
    types::{HugrSumType, HugrType, TypingSession},
};

pub trait DefHook<'c>:
    Fn(&Builder<'c>, BasicValueEnum<'c>, Node, OutgoingPort) -> Result<BasicValueEnum<'c>> + 'c
{
}

impl<
        'c,
        F: Fn(&Builder<'c>, BasicValueEnum<'c>, Node, OutgoingPort) -> Result<BasicValueEnum<'c>>
            + ?Sized
            + 'c,
    > DefHook<'c> for F
{
}

fn compose_def_hooks<'c>(
    second_hook: Rc<impl DefHook<'c> + ?Sized>,
    first_hook: Rc<impl DefHook<'c> + ?Sized>,
) -> impl DefHook<'c> {
    move |builder, value, node, port| {
        let value = second_hook(builder, value, node, port)?;
        first_hook(builder, value, node, port)
    }
}

fn leaf_def_hook<'c>(
    hook: impl Fn(&Builder<'c>, BasicValueEnum<'c>, Node, OutgoingPort) -> Result<BasicValueEnum<'c>>
        + 'c,
) -> impl DefHook<'c> {
    move |builder, value, node, port| hook(builder, value, node, port)
}

// fn sumtype_def_hook<'c, H: HugrView + 'c>(
//     sum_type: HugrSumType,
// ) -> impl Fn(&DefHookClosure<'c, H>, &Builder<'c>, BasicValueEnum<'c>) -> Result<BasicValueEnum<'c>> + 'c
// {
//     move |closure, builder, value| {
//         let llvm_sum_type =
//             LLVMSumType::try_new(closure.typing_session.as_ref(), sum_type.clone())?;
//         let sum_value = LLVMSumValue::try_new(value, llvm_sum_type.clone())?;
//         let current_block = builder.get_insert_block().unwrap();
//         let exit_block = current_block
//             .get_context()
//             .insert_basic_block_after(current_block, "");
//         let mut phis = vec![];
//         sum_value.build_destructure(builder, |builder, tag, vs| {
//             let vs = zip_eq(sum_type.get_variant(tag).unwrap().iter(), vs)
//                 .map(|(t, v)| closure.def_hook(&t.clone().try_into().unwrap(), builder, v))
//                 .collect::<Result<Vec<_>>>()?;
//             let v = llvm_sum_type.build_tag(builder, tag, vs)?;
//             phis.push((Box::new(v), builder.get_insert_block().unwrap()));
//             builder.build_unconditional_branch(exit_block)?;
//             Ok(())
//         })?;
//         builder.position_at_end(exit_block);
//         builder.build_phi(value.get_type(), "")?.add_incoming(
//             phis.iter()
//                 .map(|(v, b)| (v.as_ref() as &dyn BasicValue<'c>, *b))
//                 .collect_vec()
//                 .as_slice(),
//         );
//         Ok(value)
//     }
// }

#[derive(Default)]
pub struct DefHookClosure<'c> {
    // pub typing_session: Rc<TypingSession<'c, H>>,
    custom_hooks: HashMap<CustomType, Rc<dyn DefHook<'c> + 'c>>,
}

trait MailBoxVariantHook<'c>:
    Fn(&Builder<'c>, &[BasicValueEnum<'c>]) -> Result<Vec<BasicValueEnum<'c>>> + 'c
{
}
impl<
        'c,
        F: Fn(&Builder<'c>, &[BasicValueEnum<'c>]) -> Result<Vec<BasicValueEnum<'c>>> + ?Sized + 'c,
    > MailBoxVariantHook<'c> for F
{
}

fn either_hook<'c>(
    e: Either<impl MailBoxDefHook<'c>, impl MailBoxDefHook<'c>>,
) -> impl MailBoxDefHook<'c> {
    move |builder, value| match e {
        Either::Left(ref hook) => hook(builder, value),
        Either::Right(ref hook) => hook(builder, value),
    }
}

fn leaf_hook<'c>(
    hook: Rc<impl DefHook<'c> + ?Sized>,
    node: Node,
    port: OutgoingPort,
) -> impl MailBoxDefHook<'c> {
    move |builder, value| hook(builder, value, node, port)
}

fn variant_hook<'c>(
    field_hooks: Vec<(usize, Box<dyn MailBoxDefHook<'c> + 'c>)>,
) -> impl MailBoxVariantHook<'c> {
    move |builder, values: &[BasicValueEnum<'c>]| {
        field_hooks
            .iter()
            .map(|(i, hook)| hook(builder, values[*i]))
            .collect::<Result<Vec<_>>>()
    }
}

fn sum_hook<'c>(
    sum_type: LLVMSumType<'c>,
    variant_hooks: Vec<(usize, impl MailBoxVariantHook<'c>)>,
) -> impl MailBoxDefHook<'c> {
    let variant_hooks: HashMap<usize, _> = variant_hooks.into_iter().collect();

    move |builder, value| {
        let sum_value = LLVMSumValue::try_new(value, sum_type.clone())?;
        let current_block = builder.get_insert_block().unwrap();
        let exit_block = current_block
            .get_context()
            .insert_basic_block_after(current_block, "");
        let mut incomings = vec![];
        sum_value.build_destructure(builder, |builder, tag, mut vs| {
            if let Some(hook) = variant_hooks.get(&tag) {
                vs = hook(builder, vs.as_slice())?;
            }
            let v = sum_type.build_tag(builder, tag, vs)?;
            incomings.push((Box::new(v), builder.get_insert_block().unwrap()));
            builder.build_unconditional_branch(exit_block)?;
            Ok(())
        })?;
        builder.position_at_end(exit_block);
        let value = builder.build_phi(value.get_type(), "")?;
        value.add_incoming(
            incomings.iter()
                .map(|(v, b)| (v.as_ref() as &dyn BasicValue<'c>, *b))
                .collect_vec()
                .as_slice(),
        );
        Ok(value.as_basic_value())
    }
}

impl<'c> DefHookClosure<'c> {
    pub fn mailbox_def_hook<H: HugrView + 'c>(
        &self,
        session: &TypingSession<'c, H>,
        hugr_type: &HugrType,
        node: Node,
        port: OutgoingPort,
    ) -> Option<impl MailBoxDefHook<'c>> {
        match hugr_type.as_type_enum() {
            TypeEnum::Extension(custom_type) => self
                .custom_type_hook(custom_type, node, port)
                .map(Either::Left),
            TypeEnum::Sum(sum_type) => self
                .sum_type_hook(session, sum_type, node, port)
                .map(Either::Right),
            _ => None,
        }
        .map(either_hook)
    }

    fn custom_type_hook(
        &self,
        custom_type: &CustomType,
        node: Node,
        port: OutgoingPort,
    ) -> Option<impl MailBoxDefHook<'c>> {
        let Some(hook) = self.custom_hooks.get(custom_type) else {
            return None;
        };
        Some(leaf_hook(hook.clone(), node, port))
    }

    fn variant_hook<H: HugrView + 'c>(
        &self,
        session: &TypingSession<'c, H>,
        variant: &TypeRow,
        node: Node,
        port: OutgoingPort,
    ) -> Option<impl MailBoxVariantHook<'c>> {
        let field_hooks: Vec<(usize, Box<dyn MailBoxDefHook<'c>>)> = variant
            .iter()
            .enumerate()
            .filter_map(|(i, t)| {
                let hook = self.mailbox_def_hook(session, t, node, port)?;
                Some((i, Box::new(hook) as Box<dyn MailBoxDefHook<'c>>))
            })
            .collect_vec();

        if field_hooks.is_empty() {
            return None;
        };

        Some(variant_hook(field_hooks))
    }

    fn sum_type_hook<H: HugrView + 'c>(
        &self,
        session: &TypingSession<'c, H>,
        sum_type: &HugrSumType,
        node: Node,
        port: OutgoingPort,
    ) -> Option<impl MailBoxDefHook<'c>> {
        let llvm_sum_type = session.llvm_sum_type(sum_type.clone()).ok()?;
        let mut variant_hooks = vec![];
        for i in 0..sum_type.num_variants() {
            let typerow: TypeRow = sum_type.get_variant(i).unwrap().clone().try_into().unwrap();
            if let Some(hook) = self.variant_hook(session, &typerow, node, port) {
                variant_hooks.push((i, Box::new(hook)))
            }
        }

        if variant_hooks.is_empty() {
            return None;
        }

        Some(sum_hook(llvm_sum_type, variant_hooks))
    }

    pub fn add_leaf_hook(&mut self, hugr_type: CustomType, hook: impl DefHook<'c>) {
        use std::collections::hash_map::Entry;
        match self.custom_hooks.entry(hugr_type.into()) {
            Entry::Occupied(mut entry) => {
                let old_hook = entry.get().clone();
                entry.insert(Rc::new(compose_def_hooks(old_hook, Rc::new(hook))));
            }
            Entry::Vacant(entry) => {
                entry.insert(Rc::new(hook));
            }
        };
    }

    pub fn add_composite_hook(&mut self, hugr_type: CustomType, components: HugrSumType) {}
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
        types::BasicType,
        values::{BasicValue, CallableValue, PointerValue},
    };

    use super::DefHookClosure;

    #[test]
    fn ptr_hook() {
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

        let mut closure = DefHookClosure::default();

        closure.add_leaf_hook(ptr_custom_type, |builder, value, node, port| {
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
            let num_uses = hugr.linked_inputs(node, port).count();
            match num_uses {
                0 => {
                    builder.build_call(
                        CallableValue::try_from(fn_type.ptr_type(Default::default()).get_undef())
                            .unwrap(),
                        &[value.into(), ref_count_type.const_int(1, false).into()],
                        "",
                    )?;
                }
                x if x > 1 => {
                    builder.build_call(
                        CallableValue::try_from(fn_type.ptr_type(Default::default()).get_undef())
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
        });
    }
}
