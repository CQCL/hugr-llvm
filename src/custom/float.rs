use std::{any::TypeId, collections::HashSet};

use anyhow::{anyhow, Result};
use hugr::{
    ops::constant::CustomConst,
    std_extensions::arithmetic::float_types::{self, ConstF64},
    HugrView,
};
use inkwell::{
    types::{BasicType, FloatType},
    values::{BasicValue, BasicValueEnum},
};

use crate::emit::func::EmitFuncContext;

use super::{CodegenExtension, CodegenExtsMap};

struct FloatCodegenExtension;

impl<'c, H: HugrView> CodegenExtension<'c, H> for FloatCodegenExtension {
    fn extension(&self) -> hugr::extension::ExtensionId {
        return float_types::EXTENSION_ID;
    }

    fn llvm_type(
        &self,
        context: &crate::types::TypingSession<'c, H>,
        hugr_type: &hugr::types::CustomType,
    ) -> anyhow::Result<inkwell::types::BasicTypeEnum<'c>> {
        match hugr_type.name().as_str() {
            "float64" => Ok(context.iw_context().f64_type().as_basic_type_enum()),
            _ => todo!(),
        }
    }

    fn emitter<'a>(
        &self,
        _context: &'a mut crate::emit::func::EmitFuncContext<'c, H>,
    ) -> Box<dyn crate::emit::EmitOp<'c, hugr::ops::CustomOp, H> + 'a> {
        todo!()
    }

    fn supported_consts(&self) -> HashSet<TypeId> {
        [TypeId::of::<ConstF64>()].into_iter().collect()
    }

    fn load_constant(
        &self,
        context: &mut EmitFuncContext<'c, H>,
        konst: &dyn hugr::ops::constant::CustomConst,
    ) -> Result<Option<BasicValueEnum<'c>>> {
        let Some(k) = konst.downcast_ref::<ConstF64>() else {
            return Ok(None);
        };
        let ty: FloatType<'c> = context
            .llvm_type(&k.get_type())?
            .try_into()
            .map_err(|_| anyhow!("Failed to get ConstInt as IntType"))?;
        // TODO we don't know whether this is signed or unsigned
        Ok(Some(ty.const_float(k.value()).as_basic_value_enum()))
    }
}

pub fn add_float_extensions<H: HugrView>(cem: CodegenExtsMap<'_, H>) -> CodegenExtsMap<'_, H> {
    cem.add_cge(FloatCodegenExtension)
}

impl<H: HugrView> CodegenExtsMap<'_, H> {
    pub fn add_float_extensions(self) -> Self {
        add_float_extensions(self)
    }
}
