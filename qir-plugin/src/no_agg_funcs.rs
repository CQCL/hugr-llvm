use std::{collections::HashMap, iter};

use itertools::{zip_eq, Itertools as _};
use llvm_plugin::{inkwell::{builder::Builder, module::{Linkage, Module}, types::{ArrayType, BasicMetadataTypeEnum, BasicType, BasicTypeEnum, StructType}, values::{AnyValue, ArrayValue, BasicValue, BasicValueEnum, CallSiteValue, FunctionValue, GlobalValue, InstructionOpcode, InstructionValue, PointerValue, StructValue}, AddressSpace}, LlvmModulePass, ModuleAnalysisManager, PreservedAnalyses};

use crate::{flatten::{flatten_type, FlattenType, FlattenValue}, is_aggregate};


#[derive(Debug,Clone)]
pub struct NoAggregateFuncs;

fn is_function_applicable(func: &FunctionValue) -> bool {
    if func.get_first_basic_block().is_none() {
        return false;
    }
    // TODO likely other linkages are inapplicable too
    if func.get_linkage() != Linkage::External {
        return false;
    }

    let ty = func.get_type();
    if ty.is_var_arg() {
        return false;
    }
    ty.get_return_type().into_iter()
        .chain(ty.get_param_types().into_iter())
        .any(is_aggregate)
}

fn find_name(module: &Module, prefix: impl AsRef<str>) -> String {
    let prefix = prefix.as_ref();
    for i in 0.. {
        let name = format!("{prefix}{i}");
        if module.get_global(&name).is_none() {
            return name;
        }
    }
    unreachable!()
}

fn make_return_slot<'c>(module: &mut Module<'c>, func: FunctionValue<'c>, ty: impl BasicType<'c>) -> GlobalValue<'c> {
    let ty = ty.as_basic_type_enum();
    let func_name = func.get_name().to_str().unwrap();
    let slot_template = format!("{func_name}_return_slot");
    let slot_name = find_name(module, slot_template);
    let global =  module.add_global(ty, None, &slot_name);
    global.set_linkage(Linkage::Private);
    global.set_initializer(&match ty {
        BasicTypeEnum::FloatType(ty) => ty.get_poison().as_basic_value_enum(),
        BasicTypeEnum::IntType(ty) => ty.get_poison().as_basic_value_enum(),
        BasicTypeEnum::PointerType(ty) => ty.get_poison().as_basic_value_enum(),
        _ => panic!("make_return_slot: bad type: {ty}")
    });
    global
}

struct OldParamsToNewParams<'c> { params: Vec<(BasicValueEnum<'c>, Vec<BasicTypeEnum<'c>>)>}

impl<'c> OldParamsToNewParams<'c> {
    pub fn new(old_params: impl IntoIterator<Item=BasicValueEnum<'c>>) -> Self {
        Self { params: old_params.into_iter().map(|param| (param, flatten_type(param.get_type()).collect_vec())).collect() }
    }

    pub fn old_and_new_params(&self, new_func: FunctionValue<'c>) -> impl Iterator<Item=(BasicValueEnum<'c>,Vec<BasicValueEnum<'c>>)> + '_ {
        let mut new_params_iter = new_func.get_param_iter();
        self.params.iter().map(move |(old_param, tys)| {
            let new_params = new_params_iter.by_ref().take(tys.len()).collect_vec();
            assert!(itertools::zip_eq(tys.iter(), new_params.iter()).all(|(ty,param)| param.get_type() == *ty));
            (*old_param, new_params)
        })
    }

    pub fn new_params(&self) -> impl Iterator<Item=BasicMetadataTypeEnum<'c>> + '_ {
        self.params.iter().flat_map(|(_, tys)| tys.iter().copied().map_into())
    }
}

fn duplicate_func<'c>(module: &mut Module<'c>, old_func: FunctionValue<'c>) -> (FunctionValue<'c>, Vec<GlobalValue<'c>>) {
    let context = module.get_context();
    let func_name = old_func.get_name().to_str().unwrap();
    let old_func_ty = old_func.get_type();
    assert!(!old_func_ty.is_var_arg());
    let old_func_name = find_name(module, format!("{func_name}.old"));
    old_func.as_global_value().set_name(&old_func_name);

    let old_to_new = OldParamsToNewParams::new(old_func.get_param_iter());
    let new_func_ty = module.get_context().void_type().fn_type(old_to_new.new_params().collect_vec().as_slice(), false);
    let new_func = module.add_function(&func_name, new_func_ty, Some(old_func.get_linkage()));

    let return_slot_tys = old_func_ty.get_return_type().map_or(vec![], |ty| FlattenType::new(ty).flat_types().collect_vec());
    let return_slots = return_slot_tys.iter().map(|ty| make_return_slot(module, new_func, *ty)).collect_vec();


    eprintln!("duplicate_func: {} {} \n{} {}", old_func.get_name().to_str().unwrap(), old_func_ty, new_func.get_name().to_str().unwrap(), new_func_ty);
    // TODO copy all ancilliary data like attributes, calling convention etc

    let builder = context.create_builder();
    let entry_block = context.append_basic_block(new_func, "entry");
    builder.position_at_end(entry_block);

    for (old_param, new_params) in old_to_new.old_and_new_params(new_func) {
        let old_param = FlattenValue::from_basic_value(old_param);
        let new_v = old_param.get_type().clone().reconstitute_value(&builder, new_params);
        assert_eq!(old_param.get_type(), new_v.get_type());
        old_param.replace_all_uses_with(new_v);
    }

    builder.build_unconditional_branch(old_func.get_first_basic_block().unwrap()).unwrap();

    for bb in old_func.get_basic_blocks() {
        bb.move_after(new_func.get_last_basic_block().unwrap()).unwrap();

        let term = bb.get_terminator().unwrap();
        builder.position_at_end(bb);
        if term.get_opcode() == InstructionOpcode::Return {
            let ret_flat_vals = term.get_operands().filter_map(|x| x.and_then(|x| x.left().map(FlattenValue::from_basic_value))).collect_vec();
            let mut return_vals = vec![];
            for flat_val in ret_flat_vals {
                return_vals.extend(flat_val.flatten(&builder));
            }
            for (val, slot) in zip_eq(return_vals, return_slots.iter()) {
                builder.build_store(slot.as_pointer_value(), val).unwrap();
            }
            builder.build_return(None).unwrap();
            term.erase_from_basic_block();
        }
    }
    (new_func, return_slots)
}

// fn flatten_value<'a, 'c>(builder: &'a Builder<'c>, ret_val: BasicValueEnum<'c>) -> Box<dyn Iterator<Item=BasicValueEnum<'c>> + 'a> {
//     if let Some(struct_val) = StructValue::try_from(ret_val).ok() {
//         Box::new((0..struct_val.get_type().count_fields()).flat_map(move |i| {
//             let v = builder.build_extract_value(struct_val, i, "").unwrap();
//             flatten_value(builder, v)
//         }))
//     } else if let Some(array_val) = ArrayValue::try_from(ret_val).ok() {
//         Box::new((0..array_val.get_type().len()).flat_map(move |i| {
//             let v = builder.build_extract_value(array_val, i, "").unwrap();
//             flatten_value(builder, v)
//         }))
//     } else {
//         Box::new(iter::once(ret_val))
//     }
// }

// fn reconstitute_aggregate<'c>(builder: &Builder<'c>, agg_ty: impl BasicType<'c>, vals: impl AsRef<[BasicValueEnum<'c>]>) -> BasicValueEnum<'c> {
//     let (agg_ty, vals) = (agg_ty.as_basic_type_enum(), vals.as_ref());
//     let mut vals = vals.into_iter();
//     if let Some(struct_ty) = StructType::try_from(agg_ty).ok() {
//         // assert_eq!(vals.len(), struct_ty.count_fields() as usize);
//         let mut v = struct_ty.get_poison();
//         // let mut vals_iter = vals.into_iter();

//         for i in 0..struct_ty.count_fields() {
//             let val = reconstitute_aggregate(builder, struct_ty.get_field_type_at_index(i).unwrap(), vals.by_ref());
//             v = builder.build_insert_value(v, val, i as u32, "").unwrap().into_struct_value();
//         }
//         assert!(vals.next().is_none());
//         v.as_basic_value_enum()
//     } else if let Some(array_ty) = ArrayType::try_from(agg_ty).ok() {
//         // assert_eq!(vals.len(), array_ty.len() as usize);
//         let mut v = array_ty.get_poison();
//         // let mut vals_iter = vals.into_iter();

//         for i in 0..array_ty.len() {
//             let val = reconstitute_aggregate(builder, array_ty.get_element_type(), vals.by_ref());
//             v = builder.build_insert_value(v, val, i as u32, "").unwrap().into_array_value();
//         }
//         assert!(vals.next().is_none());
//         v.as_basic_value_enum()
//     } else {
//         // assert_eq!(vals.len(), 1);
//         vals.next().unwrap()
//     }

// }

pub fn replace_all_uses_with<'c>(from: impl BasicValue<'c>, to: impl BasicValue<'c>) {
    let (from, to) = (from.as_basic_value_enum(), to.as_basic_value_enum());
    assert_eq!(from.get_type(), to.get_type());
    match from {
        BasicValueEnum::IntValue(v) => v.replace_all_uses_with(to.into_int_value()),
        BasicValueEnum::FloatValue(v) => v.replace_all_uses_with(to.into_float_value()),
        BasicValueEnum::PointerValue(v) => v.replace_all_uses_with(to.into_pointer_value()),
        BasicValueEnum::ArrayValue(v) => v.replace_all_uses_with(to.into_array_value()),
        BasicValueEnum::StructValue(v) => v.replace_all_uses_with(to.into_struct_value()),
        _ => panic!("return wrangling: bad type: {}", from.get_type())
    }
}

fn rewrite_call<'c>(builder: &Builder<'c>, call: InstructionValue<'c>, new_func: FunctionValue<'c>, slots: impl AsRef<[GlobalValue<'c>]>) {
    assert!(new_func.get_type().get_return_type().is_none());
    let slots = slots.as_ref();
    let call_site = CallSiteValue::try_from(call).unwrap();
    builder.position_before(&call);
    assert_eq!(call_site.count_arguments() + 1, call.get_num_operands());
    let flat_args = call.get_operands().filter_map(|x| x.and_then(|x| x.left())).take(call_site.count_arguments() as usize).flat_map(|val| FlattenValue::from_basic_value(val).flatten(&builder)).map_into().collect_vec();
    // let arg_types = flat_args.iter().map(|x| x.get_type()).collect_vec();
    // assert!(new_func.get_type().get_param_types() == arg_types);
    builder.build_call(new_func, &flat_args, "").unwrap().try_as_basic_value().unwrap_right();

    if let Some(ret_v) = BasicValueEnum::try_from(call.as_any_value_enum()).ok() {
        let flatten_ret_v = FlattenValue::from_basic_value(ret_v);
        assert_eq!(flatten_ret_v.flat_len(), slots.len());
        let return_vals = slots.iter().map(|slot| builder.build_load(slot.as_pointer_value(), "").unwrap().as_basic_value_enum()).collect_vec();
        let new_ret_v = flatten_ret_v.get_type().clone().reconstitute_value(builder, return_vals);
        replace_all_uses_with(ret_v, new_ret_v);
    }
    call.erase_from_basic_block();
}

impl LlvmModulePass for NoAggregateFuncs {
    fn run_pass(
        &self,
        module: &mut Module<'_>,
        manager: &ModuleAnalysisManager,
    ) -> PreservedAnalyses {
        let mut func_slots: HashMap<FunctionValue, (FunctionValue, Vec<GlobalValue>)> = Default::default();
        for func in module.get_functions().filter(is_function_applicable) {
            let (new_func, slots) = duplicate_func(module, func);
            func_slots.insert(func, (new_func, slots));
        }

        let builder = module.get_context().create_builder();
        for func in module.get_functions() {
            for bb in func.get_basic_blocks() {
                for i in bb.get_instructions().collect_vec() {
                    let Some(call) = CallSiteValue::try_from(i).ok() else {
                        continue;
                    };
                    let Some((func,slots)) = func_slots.get(&call.get_called_fn_value()) else {
                        continue;
                    };
                    rewrite_call(&builder, i, *func, slots);
                }
            }
        }

        module.verify().unwrap();
        PreservedAnalyses::None
    }
}
