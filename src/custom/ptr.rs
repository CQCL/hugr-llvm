use std::rc::Rc;

use anyhow::{anyhow, Result};
use hugr::{
    extension::{simple_op::MakeExtensionOp as _, ExtensionId}, ops::CustomOp, std_extensions::ptr::{self, PtrOp, PtrOpDef, PTR_REG}, types::{CustomType, TypeArg}, HugrView
};
use inkwell::{
    builder::Builder, module::{Linkage, Module}, types::{BasicType, BasicTypeEnum, PointerType}, values::{BasicValueEnum, FunctionValue, PointerValue}, AddressSpace
};
use itertools::Itertools as _;

use crate::{
    emit::{EmitFuncContext, EmitModuleContext, EmitOp, EmitOpArgs},
    type_map::{def_hook::{DefHookInV, DefHookTypeMapping}, TypeMapping},
    types::TypingSession,
};

use super::{CodegenExtsMap};

struct RefCountedPtrCodegenExtension<'c>{
    inc_count: FunctionValue<'c>,
    new: FunctionValue<'c>,
}

// impl<'c, H: HugrView> CodegenExtension<'c, H> for RefCountedPtrCodegenExtension<'c> {
//     fn extension(&self) -> ExtensionId {
//         ptr::EXTENSION_ID
//     }

//     fn llvm_type(
//         &self,
//         context: &TypingSession<'c, H>,
//         hugr_type: &CustomType,
//     ) -> Result<BasicTypeEnum<'c>> {
//         if hugr_type.name() == &ptr::PTR_TYPE_ID {
//             let [TypeArg::Type { ty }] = hugr_type.args() else {
//                 return Err(anyhow!("Expected exactly one argument for ptr type"));
//             };
//             Ok(context
//                 .llvm_type(ty)?
//                 .ptr_type(AddressSpace::default())
//                 .as_basic_type_enum())
//         } else {
//             Err(anyhow!("Unsupported type: {hugr_type}"))
//         }
//     }

//     fn emitter<'a>(
//         &self,
//         context: &'a mut EmitFuncContext<'c, H>,
//     ) -> Box<dyn EmitOp<'c, CustomOp, H> + 'a> {
//         Box::new(RefCountedPtrEmitter{context, build_inc_count: build_call_inc_count(self.inc_count), new: self.new} )
//     }
// }

struct RefCountedPtrEmitter<'a, 'c, H, B> {
    context: &'a mut EmitFuncContext<'c, H>,
    build_inc_count: B,
    new: FunctionValue<'c>,
}

impl<'a, 'c,H: HugrView ,B: BuildIncCount<'c,'c>>  RefCountedPtrEmitter<'a, 'c, H, B> {
    pub fn new(context: &'a mut EmitFuncContext<'c, H>, build_inc_count: B, new: FunctionValue<'c>) -> Box<dyn EmitOp<'c,CustomOp,H> + 'a> {
        Box::new(Self { context, build_inc_count,new })
    }
}

impl<'c,H: HugrView, B: BuildIncCount<'c, 'c>> EmitOp<'c, CustomOp, H> for RefCountedPtrEmitter<'_, 'c, H, B> {
    fn emit(&mut self, args: EmitOpArgs<'c, CustomOp, H>) -> Result<()> {
        let Some(ext_op) = args.node.as_extension_op() else {
            Err(anyhow!("Not an extension op"))?
        };

        let op = PtrOp::from_extension_op(&ext_op)?;
        match op.def {
            ptr::PtrOpDef::New => {
                let builder = self.context.builder();
                let [val] = args.inputs.try_into().map_err(|_| anyhow!("PtrOpDef::New expects one input"))?;
                let val_ptr_type = val.get_type().ptr_type(Default::default());
                let expected_result_type: PointerType = {
                    let [t] = args.outputs.get_types().collect_vec().try_into().map_err(|_| anyhow!("PtrOpDef::New expects one output"))?;
                    t.try_into().map_err(|_| anyhow!("not a pointer type"))?
                };
                let size = val.get_type().size_of().ok_or(anyhow!("Could not compute size of type"))?;
                let destructor: FunctionValue = todo!();
                let allocation: PointerValue = {
                    let r = builder.build_call(self.new, &[size.into(), destructor.as_global_value().as_pointer_value().into()], "")?;
                    r.try_as_basic_value().left().and_then(|x| x.try_into().ok()).ok_or(anyhow!("PtrOpDef::New: new did not return a pointer"))?
                };
                if allocation.get_type() != val_ptr_type {
                    allocation = builder.build_bitcast(allocation, val_ptr_type, "")?.try_into().map_err(|_| anyhow!("val_ptr_type not a pointer"))?;
                }
                builder.build_store(allocation, val)?;
                if allocation.get_type() != expected_result_type {
                    allocation = builder.build_bitcast(allocation, expected_result_type, "")?.try_into().map_err(|_| anyhow!("val_ptr_type not a pointer"))?;
                }

                args.outputs.finish(builder, [allocation.into()])

            },
            ptr::PtrOpDef::Read => {
                let builder = self.context.builder();
                let [mut ptr] = args.inputs.try_into().map_err(|_| anyhow!("PtrOpDef::Read expects one input"))?;
                let [expected_type] = args.outputs.get_types().collect_vec().try_into().map_err(|_| anyhow!("PtrOpDef::Read expects one output"))?;
                let expected_ptr_type = expected_type.ptr_type(Default::default());
                if ptr.get_type() != expected_ptr_type.into() {
                    ptr = builder.build_bitcast(ptr, expected_ptr_type, "")?;
                }
                let Ok(ptr) = PointerValue::try_from(ptr) else {
                    Err(anyhow!("PtrOpDef::Read arg is not a pointer"))?
                };
                let result = builder.build_load(ptr, "")?;
                (self.build_inc_count)(builder, ptr, -1)?;
                args.outputs.finish(builder, [result])
            },
            ptr::PtrOpDef::Write => {
                let builder = self.context.builder();
                let [mut ptr, val] = args.inputs.try_into().map_err(|_| anyhow!("PtrOpDef::Write expects two inputs"))?;
                let expected_ptr_type = val.get_type().ptr_type(Default::default());
                if ptr.get_type() != expected_ptr_type.into() {
                    ptr = builder.build_bitcast(ptr, expected_ptr_type, "")?;
                }
                let Ok(ptr) = PointerValue::try_from(ptr) else {
                    Err(anyhow!("PtrOpDef::Read arg is not a pointer"))?
                };
                builder.build_store(ptr, val)?;
                (self.build_inc_count)(builder, ptr, -1)?;
                args.outputs.finish(builder, [])

            },
            _ => todo!(),
        }


    }
}

fn handle_ptr_op<'a,'c, H: HugrView, B: BuildIncCount<'c,'c>>(context: &'a mut EmitFuncContext<'c, H>, build_inc_count: B, new: FunctionValue<'c>) -> Box<dyn EmitOp<'c, CustomOp, H> + 'a> {
    RefCountedPtrEmitter::new(context, build_inc_count, new)


}

impl<'c, H: HugrView> CodegenExtsMap<'c, H> {
    pub fn add_ref_counted_ptr(self, module: &Module<'c>) -> Self {
        let ctx = module.get_context();
        let i8_ptr_type = ctx.i8_type().ptr_type(Default::default()).as_basic_type_enum();

        let inc_count_type = ctx.void_type().fn_type(&[i8_ptr_type.into(), ctx.i64_type().into()], false);
        let inc_count = module.add_function("inc_count", inc_count_type, Some(Linkage::External));
        let build_inc_count = Rc::new(build_call_inc_count(inc_count));

        let new_type = i8_ptr_type.fn_type(&[ctx.i64_type().into(), i8_ptr_type.into()], false);
        let new = module.add_function("__new", new_type, Some(Linkage::External));

        let ptr_key = (ptr::EXTENSION_ID,ptr::PTR_TYPE_ID);
        self.add_type_by_key(ptr_key.clone(), move |session, custom_type| {
            if custom_type.name() == &ptr::PTR_TYPE_ID {
                let [TypeArg::Type { ty }] = custom_type.args() else {
                    return Err(anyhow!("Expected exactly one argument for ptr type"));
                };
                Ok(session
                    .llvm_type(ty)?
                    .ptr_type(AddressSpace::default())
                    .as_basic_type_enum())
            } else {
                Err(anyhow!("Unsupported type: {custom_type}"))
            }}).add_simple_op::<PtrOpDef>(move |context| handle_ptr_op(context, build_inc_count.clone(), new))
            .set_def_hook_by_key(ptr_key, ptr_def_hook(build_call_inc_count(inc_count)))

    }
}



pub trait BuildIncCount<'a, 'c>:
    Fn(&Builder<'c>, PointerValue<'c>, isize) -> Result<()> + 'a
where
{
}

impl<'a, 'c, F: Fn(&Builder<'c>, PointerValue<'c>, isize) -> Result<()> + 'a> BuildIncCount<'a, 'c>
    for F
where
{
}

pub fn ptr_def_hook<'a, 'c, H: HugrView>(
    build_inc_count: impl BuildIncCount<'a, 'c>,
) -> impl TypeMapping<'a, DefHookTypeMapping<'a, 'c, H>> {
    let build_inc_count = Rc::new(build_inc_count);
    move |DefHookInV(_, hugr, wire)| {
        let build_inc_count = build_inc_count.clone();
        Ok(Rc::new(
            move |builder: &Builder<'c>, value: BasicValueEnum<'c>| {
                let value: PointerValue = value
                    .try_into()
                    .map_err(|_| anyhow!("no a pointer value"))?;
                let num_uses = hugr.linked_inputs(wire.node(), wire.source()).count();
                match num_uses {
                    0 => {
                        build_inc_count(builder, value.into(), -1)?;
                    }
                    x if x > 1 => {
                        build_inc_count(builder, value.into(), (x - 1) as isize)?;
                    }
                    _ => (),
                };
                Ok(value.into())
            },
        ))
    }
}

pub fn build_call_inc_count<'a, 'c>(inc_count_func: FunctionValue<'c>) -> impl BuildIncCount<'a, 'c> + 'a
where
    'c: 'a,
{
    move |builder, mut value, inc| {
        let iw_context = builder
            .get_insert_block()
            .ok_or(anyhow!("No current block"))?
            .get_context();
        let inc = iw_context.i64_type().const_int(inc as u64, true);
        let expected_ptr_type: PointerType = inc_count_func.get_type().get_param_types()[0]
            .try_into()
            .map_err(|_| anyhow!("function does not take a pointer"))?;
        let actual_ptr_type: PointerType = value.get_type().try_into()?;
        if expected_ptr_type != actual_ptr_type {
            value = builder
                .build_bitcast(value, expected_ptr_type, "")?
                .into_pointer_value();
        }
        builder.build_call(inc_count_func, &[value.into(), inc.into()], "")?;
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use crate::{
        check_emission_emit_hugr,
        emit::EmitHugr,
        test::{llvm_ctx, THugrView, TestContext},
        types::HugrType,
    };
    use hugr::{
        builder::{
            Container, Dataflow as _, DataflowSubContainer,
            HugrBuilder, ModuleBuilder,
        },
        types::Signature,
    };
    use inkwell::module::Linkage;
    use ptr::{ptr_custom_type, PTR_REG};
    use rstest::rstest;

    use super::*;

    #[rstest]
    fn ptr(mut llvm_ctx: TestContext) {
        // llvm_ctx.add_extensions(|exts| exts.add_ref_counted_ptr());
        let mut emit_context = llvm_ctx.get_emit_module_context();
        let iw_context = emit_context.iw_context();

        let inc_count_func = emit_context.module().add_function(
            "inc_count",
            iw_context.void_type().fn_type(
                &[
                    iw_context.i8_type().ptr_type(Default::default()).into(),
                    iw_context.i64_type().into(),
                ],
                false,
            ),
            Some(Linkage::External),
        );
        emit_context.add_ptr_def_hook(build_call_inc_count(inc_count_func));

        let ptr_custom_type = ptr_custom_type(HugrType::UNIT);
        let hugr = {
            let mut builder = ModuleBuilder::new();
            {
                let builder = builder
                    .define_function(
                        "main",
                        Signature::new_endo(vec![ptr_custom_type.clone().into(); 3]),
                    )
                    .unwrap();
                let [_, i2, i3] = builder.input_wires_arr();
                builder.finish_with_outputs([i2, i3, i3]).unwrap();
            }
            builder.finish_hugr(&PTR_REG).unwrap()
        };
        let emit_hugr: EmitHugr<'_, THugrView> = emit_context.into();

        check_emission_emit_hugr!(hugr, emit_hugr);
    }
}
