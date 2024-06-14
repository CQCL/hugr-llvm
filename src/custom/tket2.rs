use anyhow::{anyhow, Result};
use hugr::{
    extension::{
        prelude::{BOOL_T, QB_T},
        ExtensionId,
    },
    ops::CustomOp,
    std_extensions::arithmetic::float_types::FLOAT64_TYPE,
    type_row,
    types::FunctionType,
    HugrView,
};
use inkwell::module::Linkage;
use itertools::Itertools as _;
use std::ops::Deref;

use crate::emit::{func::EmitFuncContext, EmitOp, EmitOpArgs};

use super::{CodegenExtension, CodegenExtsMap};

struct Tket2CodegenExtension;

impl<'c, H: HugrView> CodegenExtension<'c, H> for Tket2CodegenExtension {
    fn extension(&self) -> hugr::extension::ExtensionId {
        return ExtensionId::new("quantum.tket2".to_string()).unwrap();
    }

    fn llvm_type(
        &self,
        _context: &crate::types::TypingSession<'c, H>,
        hugr_type: &hugr::types::CustomType,
    ) -> anyhow::Result<inkwell::types::BasicTypeEnum<'c>> {
        match hugr_type.name().as_str() {
            _ => todo!(),
        }
    }

    fn emitter<'a>(
        &self,
        context: &'a mut crate::emit::func::EmitFuncContext<'c, H>,
    ) -> Box<dyn crate::emit::EmitOp<'c, hugr::ops::CustomOp, H> + 'a> {
        Box::new(Tket2Emitter(context))
    }
}

struct Tket2Emitter<'c, 'd, H: HugrView>(&'d mut EmitFuncContext<'c, H>);

impl<'c, H: HugrView> EmitOp<'c, CustomOp, H> for Tket2Emitter<'c, '_, H> {
    fn emit(&mut self, args: EmitOpArgs<'c, CustomOp, H>) -> Result<()> {
        let node = args.node().generalise();
        let custom = node.deref().as_custom_op().unwrap().clone();
        let opaque = custom.into_opaque();
        match opaque.name().as_str() {
            "H" => {
                let func_type = self.0.llvm_func_type(&FunctionType::new(QB_T, QB_T))?;
                let h_func =
                    self.0
                        .module()
                        .add_function("___h", func_type, Some(Linkage::External));
                let inputs = args.inputs.into_iter().map_into().collect_vec();
                let builder = self.0.builder();
                let call = builder.build_call(h_func, inputs.as_ref(), "h_call")?;
                args.outputs
                    .finish(builder, [call.try_as_basic_value().unwrap_left()])?;
                Ok(())
            }
            "Z" => {
                let func_type = self.0.llvm_func_type(&FunctionType::new(QB_T, QB_T))?;
                let z_func =
                    self.0
                        .module()
                        .add_function("___z", func_type, Some(Linkage::External));
                let inputs = args.inputs.into_iter().map_into().collect_vec();
                let builder = self.0.builder();
                let call = builder.build_call(z_func, inputs.as_ref(), "z_call")?;
                args.outputs
                    .finish(builder, [call.try_as_basic_value().unwrap_left()])?;
                Ok(())
            }
            "X" => {
                let func_type = self.0.llvm_func_type(&FunctionType::new(QB_T, QB_T))?;
                let x_func =
                    self.0
                        .module()
                        .add_function("___x", func_type, Some(Linkage::External));
                let inputs = args.inputs.into_iter().map_into().collect_vec();
                let builder = self.0.builder();
                let call = builder.build_call(x_func, inputs.as_ref(), "x_call")?;
                args.outputs
                    .finish(builder, [call.try_as_basic_value().unwrap_left()])?;
                Ok(())
            }
            "Tdg" => {
                let func_type = self.0.llvm_func_type(&FunctionType::new(QB_T, QB_T))?;
                let x_func =
                    self.0
                        .module()
                        .add_function("___tdg", func_type, Some(Linkage::External));
                let inputs = args.inputs.into_iter().map_into().collect_vec();
                let builder = self.0.builder();
                let call = builder.build_call(x_func, inputs.as_ref(), "tdg_call")?;
                args.outputs
                    .finish(builder, [call.try_as_basic_value().unwrap_left()])?;
                Ok(())
            }
            "T" => {
                let func_type = self.0.llvm_func_type(&FunctionType::new(QB_T, QB_T))?;
                let x_func =
                    self.0
                        .module()
                        .add_function("___t", func_type, Some(Linkage::External));
                let inputs = args.inputs.into_iter().map_into().collect_vec();
                let builder = self.0.builder();
                let call = builder.build_call(x_func, inputs.as_ref(), "t_call")?;
                args.outputs
                    .finish(builder, [call.try_as_basic_value().unwrap_left()])?;
                Ok(())
            }
            "RzF64" => {
                let func_type = self
                    .0
                    .llvm_func_type(&FunctionType::new(type_row![QB_T, FLOAT64_TYPE], QB_T))?;
                let h_func =
                    self.0
                        .module()
                        .add_function("___rz", func_type, Some(Linkage::External));
                let inputs = args.inputs.into_iter().map_into().collect_vec();
                let builder = self.0.builder();
                let call = builder.build_call(h_func, inputs.as_ref(), "rz_call")?;
                args.outputs
                    .finish(builder, [call.try_as_basic_value().unwrap_left()])?;
                Ok(())
            }
            "CX" => {
                let func_type = self.0.llvm_func_type(&FunctionType::new(
                    type_row![QB_T, QB_T],
                    type_row![QB_T, QB_T],
                ))?;
                let cx_func =
                    self.0
                        .module()
                        .add_function("___cx", func_type, Some(Linkage::External));
                let inputs = args.inputs.into_iter().map_into().collect_vec();
                let builder = self.0.builder();

                let call = builder.build_call(cx_func, inputs.as_ref(), "rz_call")?;
                let call_r = call.try_as_basic_value().unwrap_left();

                let r1 = builder.build_extract_value(call_r.into_struct_value(), 0, "")?;
                let r2 = builder.build_extract_value(call_r.into_struct_value(), 1, "")?;

                args.outputs.finish(builder, [r1, r2])?;
                Ok(())
            }
            "QAlloc" => {
                let func_type = self
                    .0
                    .llvm_func_type(&FunctionType::new(type_row![], QB_T))?;
                let h_func =
                    self.0
                        .module()
                        .add_function("___qalloc", func_type, Some(Linkage::External));
                let inputs = args.inputs.into_iter().map_into().collect_vec();
                let builder = self.0.builder();
                let call = builder.build_call(h_func, inputs.as_ref(), "qalloc_call")?;
                args.outputs
                    .finish(builder, [call.try_as_basic_value().unwrap_left()])?;
                Ok(())
            }
            "QFree" => {
                let func_type = self
                    .0
                    .llvm_func_type(&FunctionType::new(QB_T, type_row![]))?;
                let h_func =
                    self.0
                        .module()
                        .add_function("___qfree", func_type, Some(Linkage::External));
                let inputs = args.inputs.into_iter().map_into().collect_vec();
                let builder = self.0.builder();
                let _call = builder.build_call(h_func, inputs.as_ref(), "qfree_call")?;
                args.outputs.finish(builder, [])?;
                Ok(())
            }
            "Measure" => {
                let func_type = self
                    .0
                    .llvm_func_type(&FunctionType::new(QB_T, type_row![QB_T, BOOL_T]))?;
                let h_func =
                    self.0
                        .module()
                        .add_function("___measure", func_type, Some(Linkage::External));
                let inputs = args.inputs.into_iter().map_into().collect_vec();
                let builder = self.0.builder();
                let call = builder.build_call(h_func, inputs.as_ref(), "measure_call")?;
                let call_r = call.try_as_basic_value().unwrap_left();
                let r1 = builder.build_extract_value(call_r.into_struct_value(), 0, "")?;
                let r2 = builder.build_extract_value(call_r.into_struct_value(), 1, "")?;
                args.outputs.finish(builder, [r1, r2])?;
                Ok(())
            }
            n => Err(anyhow!("Unknown op {}", n)),
        }
    }
}

pub fn add_tket2_extensions<H: HugrView>(cem: CodegenExtsMap<'_, H>) -> CodegenExtsMap<'_, H> {
    cem.add_cge(Tket2CodegenExtension)
}

impl<H: HugrView> CodegenExtsMap<'_, H> {
    pub fn add_tket2_extensions(self) -> Self {
        add_tket2_extensions(self)
    }
}
