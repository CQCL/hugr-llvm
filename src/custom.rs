//! Provides an interface for extending `hugr-llvm` to emit [CustomType]s,
//! [CustomConst]s, and [ExtensionOp]s.
//!
//! [CustomType]: hugr::types::CustomType
//! [CustomConst]: hugr::ops::constant::CustomConst
//! [ExtensionOp]: hugr::ops::ExtensionOp
use std::rc::Rc;

use self::extension_op::{ExtensionOpFn, ExtensionOpMap};
use hugr::{
    extension::{simple_op::MakeOpDef, ExtensionId},
    ops::{constant::CustomConst, ExtensionOp, OpName},
    HugrView,
};

use strum::IntoEnumIterator;
use types::CustomTypeKey;

use self::load_constant::{LoadConstantFn, LoadConstantsMap};
use self::types::LLVMCustomTypeFn;
use anyhow::Result;

use crate::{
    emit::{func::EmitFuncContext, EmitOpArgs},
    types::TypeConverter,
};

pub mod extension_op;
pub mod load_constant;
pub mod types;

// TODO move these extension implementations to crate::extension
// https://github.com/CQCL/hugr-llvm/issues/121
pub mod conversions;
pub mod float;
pub mod int;
pub mod logic;
pub mod prelude;

#[cfg(feature = "tket2")]
pub mod qir;

#[cfg(feature = "tket2")]
pub mod rotation;

/// A helper to register codegen extensions.
///
/// Types that implement this trait can be registered with a [CodegenExtsBuilder]
/// via [CodegenExtsBuilder::add_extension].
///
/// See [prelude::PreludeCodegenExtension] for an example.
pub trait CodegenExtension {
    /// Implementers should add each of their handlers to `builder` and return the
    /// resulting [CodegenExtsBuilder].
    fn add_extension<'a, H: HugrView + 'a>(
        self,
        builder: CodegenExtsBuilder<'a, H>,
    ) -> CodegenExtsBuilder<'a, H>
    where
        Self: 'a;
}

/// A container for a collection of codegen callbacks as they are being
/// assembled.
///
/// The callbacks are registered against several keys:
///  - [CustomType]s, with [CodegenExtsBuilder::custom_type]
///  - [CustomConst]s, with [CodegenExtsBuilder::custom_const]
///  - [ExtensionOp]s, with [CodegenExtsBuilder::extension_op]
///
/// Each callback may hold references older than `'a`.
///
/// Registering any callback silently replaces any other callback registered for
/// that same key.
///
/// [CustomType]: hugr::types::CustomType
#[derive(Default)]
pub struct CodegenExtsBuilder<'a, H> {
    load_constant_handlers: LoadConstantsMap<'a, H>,
    extension_op_handlers: ExtensionOpMap<'a, H>,
    type_converter: TypeConverter<'a>,
}

impl<'a, H: HugrView + 'a> CodegenExtsBuilder<'a, H> {
    /// Forwards to [CodegenExtension::add_extension].
    ///
    /// ```
    /// use hugr_llvm::custom::{prelude::{PreludeCodegenExtension, DefaultPreludeCodegen}, CodegenExtsBuilder};
    /// let ext = PreludeCodegenExtension::from(DefaultPreludeCodegen);
    /// CodegenExtsBuilder::<hugr::Hugr>::default().add_extension(ext);
    /// ```
    pub fn add_extension(self, ext: impl CodegenExtension + 'a) -> Self {
        ext.add_extension(self)
    }

    /// Register a callback to map a [CustomType] to a [BasicTypeEnum].
    ///
    /// [CustomType]: hugr::types::CustomType
    /// [BasicTypeEnum]: inkwell::types::BasicTypeEnum
    pub fn custom_type(
        mut self,
        custom_type: CustomTypeKey,
        handler: impl LLVMCustomTypeFn<'a>,
    ) -> Self {
        self.type_converter.custom_type(custom_type, handler);
        self
    }

    /// Register a callback to emit a [ExtensionOp], keyed by fully
    /// qualified [OpName].
    pub fn extension_op(
        mut self,
        extension: ExtensionId,
        op: OpName,
        handler: impl ExtensionOpFn<'a, H>,
    ) -> Self {
        self.extension_op_handlers
            .extension_op(extension, op, handler);
        self
    }

    /// Register callbacks to emit [ExtensionOp]s that match the
    /// definitions generated by `Op`s impl of [strum::IntoEnumIterator]>
    pub fn simple_extension_op<Op: MakeOpDef + IntoEnumIterator>(
        mut self,
        handler: impl 'a
            + for<'c> Fn(
                &mut EmitFuncContext<'c, H>,
                EmitOpArgs<'c, '_, ExtensionOp, H>,
                Op,
            ) -> Result<()>,
    ) -> Self {
        self.extension_op_handlers
            .simple_extension_op::<Op>(handler);
        self
    }

    /// Register a callback to materialise a constant implemented by `CC`.
    pub fn custom_const<CC: CustomConst>(
        mut self,
        handler: impl LoadConstantFn<'a, H, CC>,
    ) -> Self {
        self.load_constant_handlers.custom_const(handler);
        self
    }

    /// Consume `self` to return collections of callbacks for each of the
    /// supported keys.`
    pub fn finish(self) -> CodegenExtsMap<'a, H> {
        CodegenExtsMap {
            load_constant_handlers: Rc::new(self.load_constant_handlers),
            extension_op_handlers: Rc::new(self.extension_op_handlers),
            type_converter: Rc::new(self.type_converter),
        }
    }
}

/// The result of [CodegenExtsBuilder::finish]. Users are expected to
/// deconstruct this type, and consume the fields independently.
/// We expect to add further collections at a later date, and so this type is
/// marked `non_exhaustive`
#[derive(Default)]
#[non_exhaustive]
pub struct CodegenExtsMap<'a, H> {
    pub load_constant_handlers: Rc<LoadConstantsMap<'a, H>>,
    pub extension_op_handlers: Rc<ExtensionOpMap<'a, H>>,
    pub type_converter: Rc<TypeConverter<'a>>,
}
