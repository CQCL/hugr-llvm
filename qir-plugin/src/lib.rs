use std::{collections::HashMap, iter, ptr::slice_from_raw_parts, slice};

use itertools::Itertools as _;
use llvm_plugin::{
    inkwell::{types::BasicType, values::{AnyValue, AsValueRef, BasicValueEnum, FunctionValue, InstructionOpcode, InstructionValue}}, FunctionAnalysisManager, FunctionPassManager, LlvmFunctionPass, LlvmModulePass, OptimizationLevel, PassBuilder, PreservedAnalyses
};
use topo_bb::topo_bbs;

mod flatten;
mod topo_bb;
mod no_agg_funcs;

pub fn is_aggregate<'c>(ty: impl BasicType<'c>) -> bool {
    let ty = ty.as_basic_type_enum();
    ty.is_struct_type() || ty.is_array_type()
}

#[llvm_plugin::plugin(name = "qirify", version = "0.1")]
fn plugin_registrar(builder: &mut PassBuilder) {
    println!("dougrulz");
    builder.add_pipeline_start_ep_callback(|pm, _level| pm.add_pass(no_agg_funcs::NoAggregateFuncs));
    // builder.add_scalar_optimizer_late_ep_callback(|pm, _level| pm.add_pass(RealSroa));
}

struct RealSroa;

impl LlvmFunctionPass for RealSroa {
    fn run_pass(
        &self,
        function: &mut FunctionValue<'_>,
        manager: &FunctionAnalysisManager,
    ) -> PreservedAnalyses {
        println!(
            "Running RealSroa on {}",
            function.get_name().to_str().unwrap()
        );

        let Some(bbs) = topo_bbs(*function) else {
            eprintln!(
                "Function has backwards edges: {}",
                function.get_name().to_str().unwrap()
            );
            return PreservedAnalyses::All;
        };

        let mut remap = flatten::Remap::default();

        for bb in bbs {
            for instr in bb.get_instructions() {
                use llvm_plugin::inkwell::values::InstructionOpcode;
                eprintln!("{}", instr);
                match instr.get_opcode() {
                    InstructionOpcode::InsertValue => {
                        let res = BasicValueEnum::try_from(instr.as_any_value_enum()).unwrap();
                        let agg = instr.get_operand(0).unwrap().unwrap_left();
                        let val = instr.get_operand(1).unwrap().unwrap_left();
                        let idx = instr
                            .get_operand(2)
                            .unwrap()
                            .unwrap_left()
                            .into_int_value()
                            .get_zero_extended_constant()
                            .unwrap() as usize;
                        remap.insert_element(res, agg, val, idx);
                    }
                    InstructionOpcode::ExtractValue => {
                        let res = BasicValueEnum::try_from(instr.as_any_value_enum()).unwrap();
                        let agg = instr.get_operand(0).unwrap().unwrap_left();
                        let idxs = get_indices(instr);
                        if let Some(scalar) = remap.extract_element(res, agg, idxs) {
                            let mut uses = vec![];
                            let mut use_ = instr.get_first_use();
                            while let Some(u) = use_ {
                                uses.push(u);
                                use_ = u.get_next_use();
                            }
                            for u in uses {
                                if let Some(user) = InstructionValue::try_from(u.get_user()).ok() {
                                    for (i, op) in
                                        user.get_operands().enumerate().filter_map(|(i, x)| {
                                            x.and_then(|x| x.left().map(|x| (i, x)))
                                        })
                                    {
                                        if op == res {
                                            user.set_operand(i as u32, scalar);
                                        }
                                    }
                                }
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
        // let mut remap = HashMap::<BasicValueEnum, Vec<BasicValueEnum>>::new();

        // for bb in function.get_basic_blocks() {
        //     for i in bb.get_instructions() {
        //         if i.get_opcode() == llvm_plugin::inkwell::values::InstructionOpcode::InsertElement {
        //             let agg = i.get_operand(0).unwrap().unwrap_left();
        //             let val = i.get_operand(1).unwrap().unwrap_left();
        //             let idx = i.get_operand(2).unwrap().unwrap_left().into_int_value().get_zero_extended_constant().unwrap();

        //             if let Some(vecs) = remap.get_mut(&BasicValueEnum::VectorValue(vec)) {
        //                 vecs[idx] = val;
        //             } else {
        //                 let mut vecs = vec.iter().map(|v| v.into()).collect::<Vec<_>>();
        //                 vecs[idx] = val;
        //                 remap.insert(BasicValueEnum::VectorValue(vec), vecs);
        //             }
        //         }
        //     }
        // }
        PreservedAnalyses::All
    }
}

fn get_indices(
    instr: InstructionValue<'_>,
) -> Vec<usize> {
    assert!([InstructionOpcode::ExtractValue, InstructionOpcode::InsertValue].contains(&instr.get_opcode()));
    let len = unsafe { llvm_plugin::inkwell::llvm_sys::core::LLVMGetNumIndices(instr.as_value_ref()) as usize };
    let ptr = unsafe { llvm_plugin::inkwell::llvm_sys::core::LLVMGetIndices(instr.as_value_ref())};
    (unsafe { slice::from_raw_parts(ptr, len) }).iter().copied().map(|x| x as usize).collect_vec()
}

