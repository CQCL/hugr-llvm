use std::collections::HashMap;

use inkwell::values::{AggregateValue, BasicValueEnum, StructValue};
use llvm_plugin::{inkwell::values::FunctionValue, FunctionAnalysisManager, FunctionPassManager, LlvmFunctionPass, OptimizationLevel, PassBuilder, PreservedAnalyses};


#[llvm_plugin::plugin(name = "qirify", version = "0.1")]
fn plugin_registrar(builder: &mut PassBuilder) {
    builder.add_scalar_optimizer_late_ep_callback(|pm, _level| pm.add_pass(RealSroa));
}

struct RealSroa;

#[derive(Debug,Clone)]
enum Element<'c> {
    Leaf(BasicValueEnum<'c>),
    Agg(Destructured<'c>)
}

#[derive(Debug,Clone)]
struct Destructured<'c> (
    Vec<Element<'c>>
);

#[derive(Clone,Debug,Default)]
struct Remap<'c> {
    map: HashMap<BasicValueEnum<'c>, Destructured<'c>>
}

impl<'c> Remap<'c> {
    fn insert_value(&self, agg: BasicValueEnum, val: BasicValueEnum, idx: usize) {
        if let Some(destructured) = self.map.get(agg)
        if let Some(struct_val) = StructValue::try_from(agg).ok() {
            let mut fields = vec![];


        }

    }

    fn insert(&mut self, key: BasicValueEnum, val: BasicValueEnum) {
        match key {
            BasicValueEnum::VectorValue(vec) => {
                let idx = val.into_int_value().get_zero_extended_constant().unwrap();
                if let Some(vecs) = self.map.get_mut(&BasicValueEnum::VectorValue(vec)) {
                    vecs[idx] = val;
                } else {
                    let mut vecs = vec.iter().map(|v| v.into()).collect::<Vec<_>>();
                    vecs[idx] = val;
                    self.map.insert(BasicValueEnum::VectorValue(vec), vecs);
                }
            }
            _ => {}
        }
    }
}

impl LlvmFunctionPass for RealSroa {
    fn run_pass<>(
        &self,
        function: &mut FunctionValue<'_>,
        manager: &FunctionAnalysisManager,
    ) -> PreservedAnalyses {
        println!("Running RealSroa on {}", function.get_name().to_str().unwrap());

        let mut remap = HashMap::<BasicValueEnum, Vec<BasicValueEnum>>::new();

        for bb in function.get_basic_blocks() {
            for i in bb.get_instructions() {
                if i.get_opcode() == llvm_plugin::inkwell::values::InstructionOpcode::InsertElement {
                    let agg = i.get_operand(0).unwrap().unwrap_left();
                    let val = i.get_operand(1).unwrap().unwrap_left();
                    let idx = i.get_operand(2).unwrap().unwrap_left().into_int_value().get_zero_extended_constant().unwrap();

                    if let Some(vecs) = remap.get_mut(&BasicValueEnum::VectorValue(vec)) {
                        vecs[idx] = val;
                    } else {
                        let mut vecs = vec.iter().map(|v| v.into()).collect::<Vec<_>>();
                        vecs[idx] = val;
                        remap.insert(BasicValueEnum::VectorValue(vec), vecs);
                    }
                }
            }
        }
        PreservedAnalyses::All
    }
}
