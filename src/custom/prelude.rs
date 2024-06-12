use hugr::{extension::prelude, HugrView};
use inkwell::types::BasicType;

use super::{CodegenExtension, CodegenExtsMap};

struct PreludeCodegenExtension;

impl<'c, H: HugrView> CodegenExtension<'c, H> for PreludeCodegenExtension {
    fn extension(&self) -> hugr::extension::ExtensionId {
        return prelude::PRELUDE_ID
    }

    fn llvm_type(
        &self,
        context: &crate::types::TypingSession<'c, H>,
        hugr_type: &hugr::types::CustomType,
    ) -> anyhow::Result<inkwell::types::BasicTypeEnum<'c>> {
        match hugr_type.name().as_str() {
            "qubit" => Ok(context.iw_context().i32_type().as_basic_type_enum()),
            _ => todo!(),
        }
    }

    fn emitter<'a>(
        &self,
        context: &'a mut crate::emit::func::EmitFuncContext<'c, H>,
    ) -> Box<dyn crate::emit::EmitOp<'c, hugr::ops::CustomOp, H> + 'a> {
        todo!()
    }
}

pub fn add_prelude_extensions<H: HugrView>(cem: CodegenExtsMap<'_, H>) -> CodegenExtsMap<'_, H> {
    cem.add_cge(PreludeCodegenExtension)
}

impl<H: HugrView> CodegenExtsMap<'_, H> {
    pub fn add_prelude_extensions(self) -> Self {
        add_prelude_extensions(self)
    }
}
