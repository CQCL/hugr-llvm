use std::collections::HashMap;

use hugr::{extension::ExtensionId, std_extensions::ptr, types::{CustomType, TypeArg}, HugrView, Wire};
use inkwell::{types::{BasicType, BasicTypeEnum}, AddressSpace};
use anyhow::{Result,anyhow};

use crate::{emit::{func::MailBoxDefHook, EmitModuleContext}, types::TypingSession};

use super::CodegenExtension;


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

// pub fn make_refcounting_def_hooks<'c,H: HugrView>(hugr: &H, context: EmitModuleContext<'c, H>) -> Result<HashMap<Wire, MailBoxDefHook<'c>>> {
//     for (node, out_p, t) in hugr.nodes().flat_map(|n| hugr.out_value_types(n).map(move |(t, p)| (n, p, t))) {


//     }
//     todo!()
// }
