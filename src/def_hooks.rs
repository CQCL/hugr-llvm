use std::{borrow::Borrow, cell::RefCell, collections::HashMap, rc::Rc};

use anyhow::{anyhow, Result};
use hugr::{
    types::{CustomType, TypeEnum}, HugrView, Node, OutgoingPort
};
use inkwell::{
    builder::Builder,
    values::{BasicValue, BasicValueEnum},
};
use itertools::{zip_eq, Itertools};

use crate::{
    emit::func::MailBoxDefHook, sum::{LLVMSumType, LLVMSumValue}, types::{HugrSumType, HugrType, TypingSession}
};

trait DefHook<'c>: Fn(&DefHookClosure<'c>, &Builder<'c>, BasicValueEnum<'c>, Node, OutgoingPort) -> Result<BasicValueEnum<'c>> + 'c {}

impl<'c, F: Fn(&DefHookClosure<'c>, &Builder<'c>, BasicValueEnum<'c>, Node, OutgoingPort) -> Result<BasicValueEnum<'c>> + 'c> DefHook<'c> for F {}
// impl<'c, H, F: 'c> DefHook<'c, H> for F
// where
//     F: for<'a, 'b> Fn(
//         &'a DefHookClosure<'c, H>,
//         &'b Builder<'c>,
//         BasicValueEnum<'c>,
//     ) -> Result<BasicValueEnum<'c>>,
// {
//     fn def_hook(
//         &self,
//         closure: &DefHookClosure<'c, H>,
//         builder: &Builder<'c>,
//         value: BasicValueEnum<'c>,
//     ) -> Result<BasicValueEnum<'c>> {
//         self(closure, builder, value)
//     }
// }

// impl<'c, H: 'c> DefHook<'c, H> for Box<dyn DefHook<'c, H>> {
//     fn def_hook(
//         &self,
//         closure: &DefHookClosure<'c, H>,
//         builder: &Builder<'c>,
//         value: BasicValueEnum<'c>,
//     ) -> Result<BasicValueEnum<'c>> {
//         self.as_ref().def_hook(closure, builder, value)
//     }
// }

fn compose_def_hooks<'c>(
    second_hook: impl DefHook<'c>,
    first_hook: impl DefHook<'c>,
) -> impl DefHook<'c>
{
    move |closure, builder, value, node, port| {
        let value = second_hook(closure, builder, value, node, port)?;
        first_hook(closure, builder, value, node, port)
    }
}

fn leaf_def_hook<'c>(
    hook: impl Fn(&Builder<'c>, BasicValueEnum<'c>, Node, OutgoingPort) -> Result<BasicValueEnum<'c>> + 'c,
) -> impl DefHook<'c>
{
    move |_, builder, value, node, port| hook(builder, value, node, port)
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
    custom_hooks: HashMap<CustomType, Box<dyn DefHook<'c> + 'c>>,
}

impl<'c> DefHookClosure<'c> {
    pub fn mailbox_def_hook<H: HugrView>(self: Rc<Self>, session: TypingSession<'c, H>, typ: HugrType, node: Node, port: OutgoingPort) -> impl MailBoxDefHook<'c> {
        move |builder, value| {
            self.def_hook(session.clone(), &typ, builder, value, node, port)
        }
    }


    pub fn def_hook<H: HugrView>(
        &self,
        session: TypingSession<'c,H>,
        hugr_type: &HugrType,
        builder: &Builder<'c>,
        value: BasicValueEnum<'c>,
        node: Node,
        port: OutgoingPort,
    ) -> Result<BasicValueEnum<'c>> {
        match hugr_type.as_type_enum() {
            // function types can't have hooks
            TypeEnum::Function(_) => Ok(value),
            TypeEnum::Extension(custom_type) => self
                .custom_hooks
                .get(&custom_type)
                .map_or(Ok(value), |hook| hook(self, builder, value, node, port)),
            TypeEnum::Sum(sum_type) => self.sum_type_hook(session, sum_type.clone(), builder, value, node, port),
            _ => Err(anyhow!("def_hook: Unsupported type {:?}", hugr_type)),
        }
    }

    fn sum_type_hook<H: HugrView>(&self, session: TypingSession<'c, H>, sum_type: HugrSumType, builder: &Builder<'c>, value: BasicValueEnum<'c>, node: Node, port: OutgoingPort) -> Result<BasicValueEnum<'c>> {
        let llvm_sum_type = LLVMSumType::try_new(&session, sum_type.clone())?;
        let sum_value = LLVMSumValue::try_new(value, llvm_sum_type.clone())?;
        let current_block = builder.get_insert_block().unwrap();
        let exit_block = current_block
            .get_context()
            .insert_basic_block_after(current_block, "");
        let mut phis = vec![];
        sum_value.build_destructure(builder, |builder, tag, vs| {
            let vs = zip_eq(sum_type.get_variant(tag).unwrap().iter(), vs)
                .map(|(t, v)| self.def_hook(session.clone(), &t.clone().try_into().unwrap(), builder, v, node, port))
                .collect::<Result<Vec<_>>>()?;
            let v = llvm_sum_type.build_tag(builder, tag, vs)?;
            phis.push((Box::new(v), builder.get_insert_block().unwrap()));
            builder.build_unconditional_branch(exit_block)?;
            Ok(())
        })?;
        builder.position_at_end(exit_block);
        builder.build_phi(value.get_type(), "")?.add_incoming(
            phis.iter()
                .map(|(v, b)| (v.as_ref() as &dyn BasicValue<'c>, *b))
                .collect_vec()
                .as_slice(),
        );
        Ok(value)
    }

    pub fn add_leaf_hook(
        &mut self,
        hugr_type: CustomType,
        hook: impl Fn(&Builder<'c>, BasicValueEnum<'c>, Node, OutgoingPort) -> Result<BasicValueEnum<'c>> + 'c,
    ) {
        use std::collections::hash_map::Entry;
        let new_hook: Box<dyn DefHook<'c> + 'c> = Box::new(leaf_def_hook(hook));
        match self.custom_hooks.entry(hugr_type.into()) {
            Entry::Occupied(mut entry) => {
                fn dummy<'c>(
                    _: &DefHookClosure<'c>,
                    _: &Builder<'c>,
                    _: BasicValueEnum<'c>,
                    _: Node,
                    _: OutgoingPort
                ) -> Result<BasicValueEnum<'c>> {
                    unreachable!()
                }
                let mut dummy: Box<dyn DefHook<'c> + 'c> = Box::new(dummy);
                let old_hook = entry.get_mut();
                std::mem::swap(old_hook, &mut dummy);
                entry.insert(Box::new(compose_def_hooks(dummy, new_hook)));
            }
            Entry::Vacant(entry) => {
                entry.insert(new_hook);
            }
        };
    }
}

#[cfg(test)]
mod test {
    use hugr::{builder::{DFGBuilder, Dataflow, DataflowHugr, ModuleBuilder}, std_extensions::ptr::{self, ptr_custom_type, ptr_type, PTR_REG}, types::{Signature, Type}, HugrView};
    use inkwell::{types::BasicType, values::{BasicValue, CallableValue, PointerValue}};

    use super::DefHookClosure;

    #[test]
    fn ptr_hook() {
        let ptr_custom_type = ptr_custom_type(Type::UNIT);
        let hugr = {
            let builder = DFGBuilder::new(Signature::new_endo(vec![ptr_custom_type.clone().into();3])).unwrap();
            let [_,i2,i3] = builder.input_wires_arr();
            builder.finish_hugr_with_outputs([i2,i3,i3], &PTR_REG).unwrap()
        };

        let mut closure = DefHookClosure::default();

        closure.add_leaf_hook(ptr_custom_type, |builder, value, node, port| {
            let context = builder.get_insert_block().unwrap().get_context();
            let ref_count_type = context.i64_type();
            let fn_type = context.void_type().fn_type(&[value.get_type().ptr_type(Default::default()).as_basic_type_enum().into(), ref_count_type.as_basic_type_enum().into()], false);
            let num_uses = hugr.linked_inputs(node, port).count();
            match num_uses {
                0 => {
                    builder.build_call(CallableValue::try_from(fn_type.ptr_type(Default::default()).get_undef()).unwrap(), &[value.into(), ref_count_type.const_int(1, false).into()],"")?;
                },
                x if x > 1 => {
                    builder.build_call(CallableValue::try_from(fn_type.ptr_type(Default::default()).get_undef()).unwrap(), &[value.into(), ref_count_type.const_int((x - 1) as u64, false).into()],"")?;
                }
                _ => ()
            };

            Ok(value)

        });


    }

}
