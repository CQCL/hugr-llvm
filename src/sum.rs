use crate::types::{HugrSumType, TypingSession};

use anyhow::{anyhow, Result};
use hugr::types::TypeRow;
use inkwell::{
    builder::Builder,
    types::{AnyType, AsTypeRef, BasicType, BasicTypeEnum, IntType, StructType},
    values::{BasicValue, BasicValueEnum, IntValue, StructValue},
};
use itertools::{zip_eq, Itertools};
use llvm_sys_140::prelude::LLVMTypeRef;

/// The opaque representation of a hugr [SumType].
///
/// Using the public methods of this type one emit "tag"s,"untag"s, and
/// "get_tag"s while not exposing the underlying LLVM representation.
///
/// We offer impls of [BasicType] and parent traits.
#[derive(Debug)]
pub struct LLVMSumType<'c>(StructType<'c>, HugrSumType);

impl<'c> LLVMSumType<'c> {
    /// Attempt to create a new `LLVMSumType` from a [HugrSumType].
    pub fn try_new<H>(session: &TypingSession<'c, H>, sum_type: HugrSumType) -> Result<Self> {
        assert!(sum_type.num_variants() < u32::MAX as usize);
        let variants = (0..sum_type.num_variants())
            .map(|i| {
                let tr = Self::get_variant_typerow_st(&sum_type, i as u32)?;
                tr.iter()
                    .map(|t| session.llvm_type(t))
                    .collect::<Result<Vec<_>>>()
            })
            .collect::<Result<Vec<_>>>()?;
        let has_tag_field = Self::sum_type_has_tag_field(&sum_type);
        let types = has_tag_field
            .then_some(session.iw_context().i32_type().as_basic_type_enum())
            .into_iter()
            .chain(
                variants
                    .iter()
                    .map(|lty_vec| session.iw_context().struct_type(lty_vec, false).into()),
            )
            .collect_vec();
        Ok(Self(
            session.iw_context().struct_type(&types, false),
            sum_type.clone(),
        ))
    }

    /// Returns an LLVM constant value of `undef`.
    pub fn get_undef(&self) -> impl BasicValue<'c> {
        self.0.get_undef()
    }

    /// Returns an LLVM constant value of `poison`.
    pub fn get_poison(&self) -> impl BasicValue<'c> {
        self.0.get_poison()
    }

    /// Emit instructions to read the tag of a value of type `LLVMSumType`.
    ///
    /// The type of the value is that returned by [LLVMSumType::get_tag_type].
    pub fn build_get_tag(
        &self,
        builder: &Builder<'c>,
        v: impl BasicValue<'c>,
    ) -> Result<IntValue<'c>> {
        let struct_value: StructValue<'c> = v
            .as_basic_value_enum()
            .try_into()
            .map_err(|_| anyhow!("Not a struct type"))?;
        if self.has_tag_field() {
            Ok(builder
                .build_extract_value(struct_value, 0, "")?
                .into_int_value())
        } else {
            Ok(self.get_tag_type().const_int(0, false))
        }
    }

    /// Emit instructions to read the inner values of a value of type
    /// `LLVMSumType`, on the assumption that it's tag is `tag`.
    ///
    /// If it's tag is not `tag`, the returned values will be poison.
    pub fn build_untag(
        &self,
        builder: &Builder<'c>,
        tag: u32,
        v: impl BasicValue<'c>,
    ) -> Result<Vec<BasicValueEnum<'c>>> {
        debug_assert!((tag as usize) < self.1.num_variants());
        debug_assert!(v.as_basic_value_enum().get_type() == self.0.as_basic_type_enum());

        let v: StructValue<'c> = builder
            .build_extract_value(
                v.as_basic_value_enum().into_struct_value(),
                self.get_variant_index(tag),
                "",
            )?
            .into_struct_value();
        let r = (0..v.get_type().count_fields())
            .map(|i| Ok(builder.build_extract_value(v, i, "")?.as_basic_value_enum()))
            .collect::<Result<Vec<_>>>()?;
        debug_assert_eq!(r.len(), self.num_fields(tag).unwrap());
        Ok(r)
    }

    /// Emit instructions to build a value of type `LLVMSumType`, being of variant `tag`.
    pub fn build_tag(
        &self,
        builder: &Builder<'c>,
        tag: u32,
        vs: Vec<BasicValueEnum<'c>>,
    ) -> Result<BasicValueEnum<'c>> {
        let expected_num_fields = self.num_fields(tag)?;
        if expected_num_fields != vs.len() {
            Err(anyhow!("LLVMSumType::build: wrong number of fields: expected: {expected_num_fields} actual: {}", vs.len()))?
        }
        let variant_index = self.get_variant_index(tag);
        let row_t = self
            .0
            .get_field_type_at_index(variant_index)
            .ok_or(anyhow!("LLVMSumType::build: no field type at index"))
            .and_then(|row_t| {
                if !row_t.is_struct_type() {
                    Err(anyhow!("LLVMSumType::build"))?
                }
                Ok(row_t.into_struct_type())
            })?;
        debug_assert!(zip_eq(vs.iter(), row_t.get_field_types().into_iter())
            .all(|(lhs, rhs)| lhs.as_basic_value_enum().get_type() == rhs));
        let mut row_v = row_t.get_undef();
        for (i, val) in vs.into_iter().enumerate() {
            row_v = builder
                .build_insert_value(row_v, val, i as u32, "")?
                .into_struct_value();
        }
        let mut sum_v = self.get_poison().as_basic_value_enum().into_struct_value();
        if self.has_tag_field() {
            sum_v = builder
                .build_insert_value(
                    sum_v,
                    self.get_tag_type().const_int(tag as u64, false),
                    0u32,
                    "",
                )?
                .into_struct_value();
        }
        Ok(builder
            .build_insert_value(sum_v, row_v, variant_index, "")?
            .as_basic_value_enum())
    }

    /// Get the type of the value that would be returned by `build_get_tag`.
    pub fn get_tag_type(&self) -> IntType<'c> {
        self.0.get_context().i32_type()
    }

    fn sum_type_has_tag_field(st: &HugrSumType) -> bool {
        st.num_variants() >= 2
    }

    fn has_tag_field(&self) -> bool {
        Self::sum_type_has_tag_field(&self.1)
    }

    fn get_variant_index(&self, tag: u32) -> u32 {
        tag + (if self.has_tag_field() { 1 } else { 0 })
    }

    fn get_variant_typerow_st(sum_type: &HugrSumType, tag: u32) -> Result<TypeRow> {
        sum_type
            .get_variant(tag as usize)
            .ok_or(anyhow!("Bad variant index {tag} in {sum_type}"))
            .and_then(|tr| Ok(TypeRow::try_from(tr.clone())?))
    }

    fn get_variant_typerow(&self, tag: u32) -> Result<TypeRow> {
        Self::get_variant_typerow_st(&self.1, tag)
    }

    fn num_fields(&self, tag: u32) -> Result<usize> {
        let tr = self.get_variant_typerow(tag)?;
        Ok(tr.len())
    }
}

impl<'c> From<LLVMSumType<'c>> for BasicTypeEnum<'c> {
    fn from(value: LLVMSumType<'c>) -> Self {
        value.0.as_basic_type_enum()
    }
}

impl<'c> std::fmt::Display for LLVMSumType<'c> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

unsafe impl<'c> AsTypeRef for LLVMSumType<'c> {
    fn as_type_ref(&self) -> LLVMTypeRef {
        self.0.as_type_ref()
    }
}

unsafe impl<'c> AnyType<'c> for LLVMSumType<'c> {}

unsafe impl<'c> BasicType<'c> for LLVMSumType<'c> {}
