use std::{collections::HashMap, ffi::c_uint, iter::{self, Cycle}};

use itertools::Itertools as _;
use llvm_plugin::inkwell::{builder::Builder, llvm_sys::prelude::{LLVMTypeRef, LLVMValueRef}, types::{AnyType, AnyTypeEnum, ArrayType, AsTypeRef, BasicType, BasicTypeEnum, StructType}, values::{AggregateValue, AnyValue, AnyValueEnum, ArrayValue, AsValueRef, BasicValue, BasicValueEnum, StructValue}};

use anyhow::{Result};

use crate::{is_aggregate, no_agg_funcs::replace_all_uses_with};


// #[derive(Debug,Clone)]
// enum Element<'c> {
//     Leaf(BasicValueEnum<'c>),
//     Agg(Destructured<'c>)
// }

// #[derive(Debug,Clone)]
// struct Destructured<'c> (
//     Vec<Element<'c>>
// );

#[derive(Debug,Clone)]
struct Destructured<'c> {
    value: BasicValueEnum<'c>,
    flat: Vec<(usize, BasicValueEnum<'c>)>,
    once: Vec<BasicValueEnum<'c>>
}

impl<'c> Destructured<'c> {
    pub fn assert_invariants(&self) {
        assert!(self.flat.iter().all(|(i,_)| *i < self.once.len()));
        if let Some(struct_type) = StructType::try_from(self.value.get_type()).ok() {
            assert!(self.once.len() == struct_type.count_fields() as usize);
            assert!(self.once.iter().enumerate().all(|(i,v)|
                struct_type.get_field_type_at_index(i as u32).unwrap() == v.get_type()
            ));
        } else if let Some(array_type) = ArrayType::try_from(self.value.get_type()).ok() {
            assert!(self.once.len() == array_type.len() as usize);
            assert!(self.once.iter().enumerate().all(|(i,v)|
                array_type.get_element_type() == v.get_type()
            ));
        } else {
            assert_eq!(self.flat, vec![(0, self.value)] );
            assert_eq!(self.once, vec![self.value] );
        }

    }

    pub fn new(value: BasicValueEnum<'c>, once: impl IntoIterator<Item = BasicValueEnum<'c>>, flat: impl IntoIterator<Item = (usize, BasicValueEnum<'c>)>) -> Self {
        let flat = flat.into_iter().collect_vec();
        let once = once.into_iter().collect_vec();
        let r = Self { value, flat, once };
        r.assert_invariants();
        r
    }
    // pub fn iter(&self) -> impl Iterator<Item=(usize, BasicValueEnum<'c>)> + '_ {
    //     self.flat.iter().copied()
    // }
}

// impl<'c> IntoIterator for Destructured<'c> {
//     type Item = <Vec<(usize, BasicValueEnum<'c>)> as IntoIterator>::Item;
//     type IntoIter = <Vec<(usize, BasicValueEnum<'c>)> as IntoIterator>::IntoIter;

//     fn into_iter(self) -> Self::IntoIter {
//         self.elements.into_iter()
//     }
// }

#[derive(Clone,Debug,Default)]
pub struct Remap<'c> {
    map: HashMap<BasicValueEnum<'c>, Destructured<'c>>
}

fn array_element<'c>(array: ArrayValue<'c>, idx: usize) -> BasicValueEnum<'c> {
    debug_assert!(array.is_const());
    debug_assert!((array.get_type().len() as usize) < idx);
    unsafe {
        // newer LLVMs would use LLVMGetAggregateElement
        #[cfg(feature = "llvm14-0")]
        use llvm_plugin::inkwell::llvm_sys::core::LLVMGetElementAsConstant as get_aggregate_element;

        BasicValueEnum::new(get_aggregate_element(array.as_value_ref(), idx as c_uint).into())
    }
}


impl<'c> Remap<'c> {
    pub fn flatten(&self, agg: impl BasicValue<'c>) -> Box<dyn Iterator<Item=(usize, BasicValueEnum<'c>)> + '_>{
        let agg = agg.as_basic_value_enum();
        if let Some(struct_value) = StructValue::try_from(agg).ok() {
            if struct_value.is_const() {
                Box::new(struct_value.get_fields().enumerate().flat_map(|(i,v)| {
                    self.flatten(v).map(move |(_, v)| (i, v))
                }))
            } else if let Some(vs) = self.map.get(&agg) {
                Box::new(vs.flat.iter().copied())
            } else {
                panic!("Unknown value: {agg}")
            }
        } else if let Some(array_value) = ArrayValue::try_from(agg).ok() {
            if array_value.is_const() {
                Box::new((0..(array_value.get_type().len() as usize)).flat_map(move |i|
                                                                 self.flatten(array_element(array_value, i))
                                                                     .map(move |(_, v)| (i, v))))
            } else if let Some(vs) = self.map.get(&agg) {
                Box::new(vs.flat.iter().copied())
            } else {
                panic!("Unknown value: {agg}")
            }
        } else {
            Box::new(iter::once((0, agg)))
        }
    }

    pub fn once(&self, agg: impl BasicValue<'c>) -> Box<dyn Iterator<Item = BasicValueEnum<'c>> + '_> {
        let agg = agg.as_basic_value_enum();
        if let Some(struct_value) = StructValue::try_from(agg).ok() {
            if struct_value.is_const() {
                Box::new(struct_value.get_fields())
            } else if let Some(vs) = self.map.get(&agg) {
                Box::new(vs.once.iter().copied())
            } else {
                panic!("Unknown value: {agg}")
            }
        } else if let Some(array_value) = ArrayValue::try_from(agg).ok() {
            if array_value.is_const() {
                Box::new((0..(array_value.get_type().len() as usize)).map(move |i| array_element(array_value, i)))
            } else if let Some(vs) = self.map.get(&agg) {
                Box::new(vs.once.iter().copied())
            } else {
                panic!("Unknown value: {agg}")
            }
        } else {
            Box::new(iter::once(agg))
        }
    }


    pub fn insert_element(&mut self, res: impl BasicValue<'c>, agg: impl BasicValue<'c>, val: impl BasicValue<'c>, idx: usize) {
        let (res,agg,val) = (res.as_basic_value_enum(), agg.as_basic_value_enum(), val.as_basic_value_enum());
        let destructured = {
            debug_assert!(!self.map.contains_key(&res.as_basic_value_enum()));
            let once  = self.once(agg).collect_vec();
            let mut flat_agg_iter = self.flatten(val).map(|x| x.1);
            let vs = self.flatten(agg).map(|(i, v)| (i, if i == idx { flat_agg_iter.next().unwrap() } else { v })).collect_vec();
            debug_assert!(flat_agg_iter.next().is_none());
            Destructured::new(res, once, vs)
        };
        self.map.insert(res.as_basic_value_enum(), destructured);
    }

    pub fn extract_element(&mut self, res: impl BasicValue<'c>, agg: impl BasicValue<'c>, idxs: impl IntoIterator<Item=usize>) -> Option<BasicValueEnum<'c>>{
        let (res,agg) = (res.as_basic_value_enum(), agg.as_basic_value_enum());
        let mut val = agg;
        for idx in idxs {
            val = self.once(val).nth(idx).unwrap();
        }
        let once = self.once(val).collect_vec();
        let flat = self.flatten(val).collect_vec();
        self.map.insert(res.as_basic_value_enum(), Destructured::new(res, once, flat));
        if val.get_type().is_struct_type() || val.get_type().is_array_type() {
            Some(val)
        }
        else {
            None
        }
    }

}

pub fn flatten_type<'c>(ty: impl BasicType<'c>) -> Box<dyn Iterator<Item=BasicTypeEnum<'c>> + 'c> {
    let ty = ty.as_basic_type_enum();
    if let Some(struct_type) = StructType::try_from(ty).ok() {
        Box::new(struct_type.get_field_types_iter().flat_map(flatten_type))
    } else if let Some(array_type) = ArrayType::try_from(ty).ok() {
        let elem_tys = flatten_type(array_type.get_element_type()).collect_vec();
        let len = elem_tys.len();
        Box::new(elem_tys.into_iter().cycle().take(len * array_type.len() as usize))
    } else {
        Box::new(iter::once(ty))
    }
}

pub fn flatten_type2<'c>(ty: impl BasicType<'c>) -> Box<dyn Iterator<Item=FlattenType<'c>> + 'c> {
    let ty = ty.as_basic_type_enum();
    if let Some(struct_type) = StructType::try_from(ty).ok() {
        Box::new(struct_type.get_field_types_iter().flat_map(flatten_type2))
    } else if let Some(array_type) = ArrayType::try_from(ty).ok() {
        let elem_tys = flatten_type2(array_type.get_element_type()).collect_vec();
        let len = elem_tys.len();
        Box::new(elem_tys.into_iter().cycle().take(len * array_type.len() as usize))
    } else {
        Box::new(iter::once(FlattenType::Leaf(ty)))
    }
}


#[derive(Debug,Clone, PartialEq,Eq)]
pub enum FlattenType<'c> {
    Leaf(BasicTypeEnum<'c>),
    Struct(StructType<'c>, usize, Vec<FlattenType<'c>>),
    Array(ArrayType<'c>, Box<FlattenType<'c>>)
}

impl<'c> FlattenType<'c> {
    pub fn new(ty: impl BasicType<'c>) -> Self {
        let ty = ty.as_basic_type_enum();
        if let Some(struct_ty) = StructType::try_from(ty).ok() {

            let flat_tys = struct_ty.get_field_types_iter().map(FlattenType::new).collect_vec();
            let len = flat_tys.iter().map(|x| x.flat_len()).sum();
            Self::Struct(struct_ty, len, flat_tys)
        } else if let Some(array_ty) = ArrayType::try_from(ty).ok() {
            Self::Array(array_ty, Box::new(FlattenType::new(array_ty.get_element_type())))
        } else {
            Self::Leaf(ty)
        }
    }

    pub fn get_type(&self) -> BasicTypeEnum<'c> {
        match self {
            FlattenType::Leaf(ty) => *ty,
            FlattenType::Struct(ty, ..) => ty.as_basic_type_enum(),
            FlattenType::Array(ty, ..) => ty.as_basic_type_enum(),
        }
    }

    pub fn flat_len(&self) -> usize {
        match self {
            FlattenType::Leaf(_) => 1,
            FlattenType::Struct(_, l, ..) => *l,
            FlattenType::Array(ty, el_ty) => el_ty.flat_len() * ty.len() as usize,
        }
    }

    // pub fn sub_types(&self) -> impl Iterator<Item = FlattenType<'c>>

    pub fn reconstitute_value(self, builder: &Builder<'c>, vals: impl AsRef<[BasicValueEnum<'c>]>) -> FlattenValue<'c> {
        let vals = vals.as_ref();
        assert_eq!(self.flat_len(), vals.len());
        let v = match &self {
            FlattenType::Leaf(ty) => {
                vals[0]
            }
            FlattenType::Struct(struct_ty, _, field_flat_tys) => {
                let mut v = struct_ty.get_poison();
                let mut field_start = 0;
                for (i, field_flat_ty) in field_flat_tys.iter().enumerate() {
                    let field_end = field_start + field_flat_ty.flat_len();
                    let field_val = field_flat_ty.clone().reconstitute_value(builder, &vals[field_start..field_end]);
                    v = builder.build_insert_value(v, field_val, i as u32, "").unwrap().into_struct_value();
                    field_start = field_end;
                }
                v.as_basic_value_enum()
            },
            FlattenType::Array(array_ty, elem_flat_ty) => {
                let mut v = array_ty.get_poison();
                let mut field_start = 0;
                for i in 0..array_ty.len() {
                    let field_end = field_start + elem_flat_ty.flat_len();
                    let val = elem_flat_ty.clone().reconstitute_value(builder, &vals[field_start..field_end]);
                    v = builder.build_insert_value(v, val, i as u32, "").unwrap().into_array_value();
                    field_start = field_end;
                }
                v.as_basic_value_enum()
            }
        };
        FlattenValue::new(self,v)
    }

    fn flatten_value(&self, builder: &Builder<'c>, v: impl BasicValue<'c>) -> Vec<BasicValueEnum<'c>> {
        let v = v.as_basic_value_enum();
        eprintln!("flatten_value: {v}");
        assert_eq!(self.get_type(), v.get_type());
        match self {
            FlattenType::Leaf(_) => vec![v],
            FlattenType::Struct(_, _, field_flat_tys) => {
                let v = v.into_struct_value();
                let mut r = vec![];
                for (i, flat_ty) in field_flat_tys.iter().enumerate() {
                    let field_v = builder.build_extract_value(v, i as u32, "").unwrap();
                    assert_eq!(flat_ty.as_basic_type_enum(), field_v.get_type());
                    r.extend(flat_ty.flatten_value(builder, builder.build_extract_value(v, i as u32, "").unwrap()));
                }
                r
            },
            FlattenType::Array(array_ty, elem_flat_ty) => {
                let v = v.into_array_value();
                let mut r = vec![];
                for i in 0..array_ty.len() {
                    r.extend(elem_flat_ty.flatten_value(builder, builder.build_extract_value(v, i as u32, "").unwrap()));
                }
                r
            },
        }
    }

    pub fn flat_types(&self) -> Box<dyn Iterator<Item=BasicTypeEnum<'c>> + '_> {
        match self {
            FlattenType::Leaf(t) => Box::new(iter::once(*t)),
            FlattenType::Struct(_, _, flat_tys) => Box::new(flat_tys.iter().flat_map(Self::flat_types)),
            FlattenType::Array(array_ty, elem_flat_ty) => Box::new((0..array_ty.len()).flat_map(|_| elem_flat_ty.flat_types()))
        }
    }
}

unsafe impl AsTypeRef for FlattenType<'_> {
    fn as_type_ref(&self) -> LLVMTypeRef {
        self.as_basic_type_enum().as_type_ref()
    }
}

unsafe impl <'c> AnyType<'c> for FlattenType<'c> {
    fn as_any_type_enum(&self) -> AnyTypeEnum<'c> {
        self.as_basic_type_enum().as_any_type_enum()
    }
}

unsafe impl<'c> BasicType<'c> for FlattenType<'c> {
    fn as_basic_type_enum(&self) -> BasicTypeEnum<'c> {
        match self {
            FlattenType::Leaf(t) => *t,
            FlattenType::Struct(t, ..) => t.as_basic_type_enum(),
            FlattenType::Array(t, ..) => t.as_basic_type_enum(),
        }
    }
}


#[derive(Debug,Clone)]
pub struct FlattenValue<'c>(BasicValueEnum<'c>,FlattenType<'c>);

impl<'c> FlattenValue<'c> {
    pub fn new(ty: FlattenType<'c>, value: impl BasicValue<'c>) -> Self {
        let value = value.as_basic_value_enum();
        assert_eq!(ty.get_type(), value.get_type());
        Self(value, ty)
    }

    pub fn get_type(&self) -> &FlattenType<'c> {
        &self.1
    }

    pub fn flat_len(&self) -> usize {
        self.get_type().flat_len()
    }

    pub fn from_basic_value(v: impl BasicValue<'c>) -> Self {
        Self::new(FlattenType::new(v.as_basic_value_enum().get_type()),v)
    }

    pub fn flatten(&self, builder: &Builder<'c>) -> Vec<BasicValueEnum<'c>> {
        self.get_type().flatten_value(builder, self.as_basic_value_enum())
    }

    pub fn replace_all_uses_with(self, v: impl BasicValue<'c>) {
        let v = v.as_basic_value_enum();
        assert_eq!(self.get_type().as_basic_type_enum(), v.get_type());
        replace_all_uses_with(self, v)
    }
}

unsafe impl AsValueRef for FlattenValue<'_> {
    fn as_value_ref(&self) -> LLVMValueRef {
        self.0.as_value_ref()
    }
}

unsafe impl<'c> AnyValue<'c> for FlattenValue<'c> {
    fn as_any_value_enum(&self) -> AnyValueEnum<'c> {
        self.0.as_any_value_enum()
    }
}

unsafe impl<'c> BasicValue<'c> for FlattenValue<'c> {
    fn as_basic_value_enum(&self) -> BasicValueEnum<'c> {
        self.0
    }
}

#[cfg(test)]
mod test {
    use itertools::Itertools as _;
    use llvm_plugin::inkwell::{context::Context, types::{BasicType, BasicTypeEnum}};

    use crate::flatten::FlattenType;

    #[test]
    fn flatten_type() {
        let ctx = Context::create();
        let i32_ty = ctx.i32_type().as_basic_type_enum();
        let struct_i32_ty = ctx.struct_type(&[i32_ty], false).as_basic_type_enum();
        let struct_2_i32_ty = ctx.struct_type(&[i32_ty,i32_ty], false).as_basic_type_enum();
        let arr_i32_ty = i32_ty.array_type(3).as_basic_type_enum();

        let struct_struct_ty = ctx.struct_type(&[struct_i32_ty, arr_i32_ty], false).as_basic_type_enum();

        let cases = [(i32_ty, vec![i32_ty]),
                     (struct_i32_ty, vec![i32_ty]),
                     (struct_2_i32_ty, vec![i32_ty, i32_ty]),
                     (arr_i32_ty, vec![i32_ty, i32_ty, i32_ty]),
                     (struct_struct_ty, vec![i32_ty, i32_ty, i32_ty, i32_ty]),
        ];

        for (lhs, rhs) in cases {
            assert_eq!(FlattenType::new(lhs).flat_types().collect_vec(),rhs)
        }
    }
}
