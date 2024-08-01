use std::{collections::HashMap, rc::Rc};

use hugr::{extension::ExtensionId, std_extensions::ptr, types::{CustomType, TypeArg}, HugrView, Wire};
use inkwell::{builder::Builder, types::{BasicType, BasicTypeEnum, PointerType}, values::{BasicValueEnum, FunctionValue, PointerValue}, AddressSpace};
use anyhow::{Result,anyhow};

use crate::{emit::{func::MailBoxDefHook, EmitModuleContext}, type_map::{def_hook::{DefHookInV, DefHookTypeMap, DefHookTypeMapping}, TypeMapping}, types::TypingSession};

use super::{CodegenExtension, CodegenExtsMap};


struct RefCountedPtrCodegenExtension;

impl<'c,H> CodegenExtension<'c,H> for RefCountedPtrCodegenExtension {
    fn extension(&self) -> ExtensionId {
        ptr::EXTENSION_ID
    }

    fn llvm_type(
        &self,
        context: &TypingSession<'c, H>,
        hugr_type: &CustomType,
    ) -> Result<BasicTypeEnum<'c>> {
        if hugr_type.name() == &ptr::PTR_TYPE_ID {
            let [TypeArg::Type {
                ty,
            }] = hugr_type.args() else {
                return Err(anyhow!("Expected exactly one argument for ptr type"));
            };
            Ok(context.llvm_type(ty)?.ptr_type(AddressSpace::default()).as_basic_type_enum())
        } else {
            Err(anyhow!("Unsupported type: {hugr_type}"))
        }
    }

    fn emitter<'a>(
        &self,
        context: &'a mut crate::emit::EmitFuncContext<'c, H>,
    ) -> Box<dyn crate::emit::EmitOp<'c, hugr::ops::CustomOp, H> + 'a> {
        todo!()
    }
}

impl<'c,H> CodegenExtsMap<'c,H> {
    pub fn add_ref_counted_ptr(self) -> Self {
        self.add_cge(RefCountedPtrCodegenExtension)
    }
}

pub trait BuildIncCount<'a,'c> : Fn(&Builder<'c>, PointerValue<'c>, isize) -> Result<()> + 'a where 'c: 'a{}

impl<'a, 'c, F: Fn(&Builder<'c>, PointerValue<'c>, isize) -> Result<()> + 'a> BuildIncCount<'a,'c> for F where 'c: 'a{}

pub fn add_ptr_def_hook<'c,H: HugrView>(context: &mut EmitModuleContext<'c,H>, build_inc_count: impl BuildIncCount<'c,'c>) {
    let build_inc_count = Rc::new(build_inc_count);
    context.set_def_hook((ptr::EXTENSION_ID, ptr::PTR_TYPE_ID), move |DefHookInV(_, hugr, wire)| {
        let build_inc_count = build_inc_count.clone();
        Ok(Rc::new(move |builder: &Builder<'c>, value: BasicValueEnum<'c>| {
            let value: PointerValue = value.try_into().map_err(|_| anyhow!("no a pointer value"))?;
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
        }))
    })
}

pub fn build_call_inc_count<'a, 'c>(inc_count_func: FunctionValue<'c>) -> impl BuildIncCount<'a,'c> where 'c: 'a{
    move |builder, mut value, inc| {
        let iw_context = builder.get_insert_block().ok_or(anyhow!("No current block"))?.get_context();
        let inc = iw_context.i64_type().const_int(inc as u64, true);
        let expected_ptr_type: PointerType = inc_count_func.get_type().get_param_types()[0].try_into().map_err(|_| anyhow!("function does not take a pointer"))?;
        let actual_ptr_type: PointerType = value.get_type().try_into()?;
        if expected_ptr_type != actual_ptr_type {
            value = builder.build_bitcast(value, expected_ptr_type, "")?.into_pointer_value();
        }
        builder.build_call(inc_count_func, &[value.into(), inc.into()], "")?;
        Ok(())
    }
}

impl<'c, H: HugrView> EmitModuleContext<'c, H> {
    pub fn add_ptr_def_hook(&mut self, build_inc_count: impl BuildIncCount<'c,'c>) {
        add_ptr_def_hook(self, build_inc_count)
    }
}

#[cfg(test)]
mod test {
    use hugr::{builder::{Container, DFGBuilder, Dataflow as _, DataflowHugr as _, DataflowSubContainer, HugrBuilder, ModuleBuilder}, types::{Signature, Type}};
    use ptr::{ptr_custom_type, PTR_REG};
    use rstest::rstest;
    use crate::{check_emission, check_emission_emit_hugr, emit::{EmitHugr, EmitOp}, test::{llvm_ctx, THugrView, TestContext}, type_map::def_hook::DefHookTypeMap, types::HugrType};
    use inkwell::module::Linkage;

    use super::*;

    #[rstest]
    fn ptr(mut llvm_ctx: TestContext) {
        llvm_ctx.add_extensions(|exts| exts.add_ref_counted_ptr());
        let mut emit_context = llvm_ctx.get_emit_module_context();
        let iw_context = emit_context.iw_context();

        let inc_count_func = emit_context.module().add_function("inc_count", iw_context.void_type().fn_type(&[iw_context.i8_type().ptr_type(Default::default()).into(),iw_context.i64_type().into()], false), Some(Linkage::External));
        emit_context.add_ptr_def_hook(build_call_inc_count(inc_count_func));

        let ptr_custom_type = ptr_custom_type(HugrType::UNIT);
        let hugr = {
            let mut builder = ModuleBuilder::new();
            {
                let builder = builder.define_function("main", Signature::new_endo(vec![ptr_custom_type.clone().into(); 3])).unwrap();
                let [_, i2, i3] = builder.input_wires_arr();
                builder
                    .finish_with_outputs([i2, i3, i3])
                    .unwrap();
            }
            builder.finish_hugr(&PTR_REG).unwrap()
        };
        let emit_hugr: EmitHugr<'_,THugrView> = emit_context.into();

        check_emission_emit_hugr!(hugr, emit_hugr);
    }
}
