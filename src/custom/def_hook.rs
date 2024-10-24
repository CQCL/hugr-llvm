use std::{marker::PhantomData, rc::Rc};

use anyhow::{anyhow, bail, Result};
use hugr::{ops::OpType, types::{CustomType, TypeRow}, HugrView, Node, OutgoingPort, Wire};
use inkwell::{basic_block::BasicBlock, builder::Builder, types::BasicType, values::{BasicValue, BasicValueEnum}};
use itertools::{zip_eq, Itertools as _};

use crate::{emit::EmitFuncContext, sum::{LLVMSumType, LLVMSumValue}, types::{HugrFuncType, HugrSumType, HugrType, TypingSession}, utils::{fat::FatNode, type_map::{TypeMap, TypeMapping, TypeMappingFn}}};

use super::types::CustomTypeKey;

pub trait DefHookFn<H>
{
    fn def_hook<'c>(&self, ctx: &mut EmitFuncContext<'c,'_,H>, h: &H, w: Wire, v: BasicValueEnum<'c>) -> Result<BasicValueEnum<'c>>;
}
// impl<H, F: for<'c> Fn(&mut EmitFuncContext<'c,'_,H>, &H, Wire, BasicValueEnum<'c>) -> Result<BasicValueEnum<'c>> + ?Sized> DefHookFn<H> for F {}




pub struct DefHook<'a,H>(Option<Box<dyn for<'c> Fn(&mut EmitFuncContext<'c,'_,H>, &H, Wire, BasicValueEnum<'c>) -> Result<BasicValueEnum<'c>> + 'a>>);

impl<'a, H> DefHook<'a,H> {
    fn is_none(&self) -> bool { self.0.is_none() }

    fn new(f: impl for<'c> Fn(&mut EmitFuncContext<'c,'_,H>, &H, Wire, BasicValueEnum<'c>) -> Result<BasicValueEnum<'c>> + 'a) -> Self {
        Self(Some(Box::new(f)))
    }
}
// pub trait SumHookFn:
//     for<'c> Fn(&Builder<'c>, LLVMSumValue<'c>) -> Result<LLVMSumValue<'c>>
// {
// }

// impl<F: for<'c> Fn(&Builder<'c>, LLVMSumValue<'c>) -> Result<LLVMSumValue<'c>> + ?Sized> SumHookFn for F {}

#[derive(Default, Clone)]
pub struct DefHookTypeMapping<H>(PhantomData<H>);

fn sum_handler<'a, 'b: 'a, H: HugrView + 'b>(sum_type: HugrSumType, variants: Vec<Vec<DefHook<'a,H>>>) -> DefHook<'a,H>{
    let variants = variants.into_iter().collect_vec();
    if variants.iter().all(|v| v.iter().all(DefHook::is_none)) {
        return DefHook(None)
    }
    let hugr_type = HugrType::from(sum_type.clone());
    DefHook(Some(Box::new(move |context, hugr, wire, v| {
        let llvm_sum_type = context.llvm_sum_type(sum_type.clone())?;
        let sum_val = LLVMSumValue::try_new(v, llvm_sum_type.clone())?;
        let exit_bb = context.new_basic_block("",None);
        let exit_rmb = context.new_row_mail_box([&hugr_type],"")?;

        let variant_bbs = variants.iter().enumerate().map(|(tag, row_hooks)| {
            let row = TypeRow::try_from(sum_type.get_variant(tag).unwrap().clone())?;
            let rmb = context.new_row_mail_box(row.iter(), "")?;
            context.build_positioned_new_block("", Some(exit_bb), |context, bb| {
                let mut fields = rmb.read_vec(context.builder(),[])?;
                for (i, field_value) in fields.iter_mut().enumerate()  {
                    if let DefHook(Some(hook)) = &row_hooks[i] {
                        *field_value = (hook.as_ref())(context, hugr, wire, *field_value)?;
                    }
                }
                let r = llvm_sum_type.build_tag(context.builder(), tag, fields)?;
                exit_rmb.write(context.builder(), [r.as_basic_value_enum()])?;
                context.builder().build_unconditional_branch(exit_bb)?;
                Ok((bb,rmb))
            })
        }).collect::<Result<Vec<_>>>()?;

        sum_val.build_destructure(context.builder(), |builder, tag, variant_vals| {
            let (bb, rmb) = &variant_bbs[tag];
            rmb.write(builder, variant_vals)?;
            builder.build_unconditional_branch(*bb)?;
            Ok(())
        })?;

        context.builder().position_at_end(exit_bb);
        let r = exit_rmb.read_vec(context.builder(), [])?[0];
        Ok(r)
    })))
}

impl<'a, H: HugrView + 'a> TypeMapping<'a> for DefHookTypeMapping<H> {
    type InV<'b> = ()
    where 'b: 'a
        ;

    type OutV<'b> = DefHook<'b, H> where 'b: 'a ;

    type SumOutV<'b> = Self::OutV<'b>
    where Self: 'b
    ;

    type FuncOutV<'b> = Self::OutV<'b>
    where 'b: 'a
        ;

    fn sum_into_out<'b: 'a>(&self, sum: Self::SumOutV<'b>) -> Self::OutV<'b>{
        sum
    }

    fn func_into_out<'b: 'a>(&self, func: Self::FuncOutV<'b>) -> Self::OutV<'b>{
        func
    }

    fn default_out<'b: 'a>(
        &self,
        _: Self::InV<'b>,
        _: &crate::types::HugrType,
    ) -> Result<Self::OutV<'b>>{
        Ok(DefHook(None))
    }

    fn map_sum_type<'b: 'a>(
        &self,
        sum_type: &HugrSumType,
        wire: (),
        variants: impl IntoIterator<Item = Vec<Self::OutV<'b>>>,
    ) -> Result<Self::SumOutV<'b>>  where H: 'b{
        Ok(sum_handler(sum_type.clone(), variants.into_iter().collect()))
    }

    fn map_function_type<'b: 'a>(
        &self,
        _: &HugrFuncType,
        _context: Self::InV<'b>,
        _inputs: impl IntoIterator<Item = Self::OutV<'b>>,
        _outputs: impl IntoIterator<Item = Self::OutV<'b>>,
    ) -> Result<Self::FuncOutV<'b>>  {
        Ok(DefHook(None))
    }
}

#[derive(Default)]
pub struct DefHookMap<'a,H: HugrView + 'a>(TypeMap<'a, DefHookTypeMapping<H>>);

// fn hook<'a, H: HugrView + 'a>(node: FatNode<'a, OpType, H>, wire: Wire, custom_type: CustomType, handler: Rc<impl for<'c> Fn(&mut EmitFuncContext<'c,'_,H>, &H, Wire, &CustomType, BasicValueEnum<'c>) -> Result<BasicValueEnum<'c>> + 'a>) -> DefHook<'a,H>  {
//     let custom_type = custom_type.clone();
//     DefHook(Some(Box::new(move |context, value| handler(context, node.hugr(), wire, &custom_type, value))))

// }

// fn callback<'b, H: HugrView + 'b>(handler: Rc<impl for<'c> Fn(&mut EmitFuncContext<'c,'_,H>, &H, Wire, &CustomType, BasicValueEnum<'c>) -> Result<BasicValueEnum<'c>> + 'b>) -> impl TypeMappingFn<DefHookTypeMapping<H>> + 'b {
//     move |(fat_node, wire),custom_type: &CustomType| Ok(hook(fat_node,wire,custom_type.clone(), handler.clone()))
// }

// impl<'a,H: HugrView + 'a> DefHookMap<'a,H> {
//     pub fn def_hook(&mut self, custom_type: CustomTypeKey, handler: impl for<'c,'hugr> Fn(&mut EmitFuncContext<'c,'_,H>, &'hugr H, Wire, &CustomType, BasicValueEnum<'c>) -> Result<BasicValueEnum<'c>> + 'a) {
//         let handler = Rc::new(handler);
//         self.0.set_callback(custom_type, Box::new(move |(fat_node, wire),custom_type| Ok(hook(fat_node,wire,custom_type,handler.clone()))));
//     }



// }
