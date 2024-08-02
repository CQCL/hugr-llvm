use anyhow::anyhow;
use hugr::{
    extension::prelude::{ConstExternalSymbol, ConstUsize, PRELUDE_ID, QB_T, USIZE_T},
    ops::constant::CustomConst,
    types::TypeEnum,
    HugrView,
};
use inkwell::types::{BasicType, IntType};

use crate::{types::TypingSession};

use super::CodegenExtensionsBuilder;


/// A helper trait for implementing [CodegenExtension]s for
/// [hugr::extension::prelude].
///
/// All methods have sensible defaults provided, and [DefaultPreludeCodegen] is
/// trivial implementation o this trait, which delegates everything to those
/// default implementations.
///
/// One should use either [PreludeCodegenExtension::new], or
/// [CodegenExtsMap::add_prelude_extensions] to work with the
/// [CodegenExtension].
///
/// TODO several types and ops are unimplemented. We expect to add methods to
/// this trait as necessary, allowing downstream users to customise the lowering
/// of `prelude`.
pub trait PreludeCodegen: Clone {
    /// Return the llvm type of [hugr::extension::prelude::USIZE_T]. That type
    /// must be an [IntType].
    fn usize_type<'c, H>(&self, session: &TypingSession<'c, H>) -> IntType<'c> {
        session.iw_context().i64_type()
    }

    /// Return the llvm type of [hugr::extension::prelude::QB_T].
    fn qubit_type<'c, H>(&self, session: &TypingSession<'c, H>) -> impl BasicType<'c> {
        session.iw_context().i16_type()
    }
}

/// A trivial implementation of [PreludeCodegen] which passes all methods
/// through to their default implementations.
#[derive(Default, Clone)]
pub struct DefaultPreludeCodegen;

impl PreludeCodegen for DefaultPreludeCodegen {}

/// Add a [PreludeCodegenExtension] to the given [CodegenExtsMap] using `pcg`
/// as the implementation.
pub fn add_prelude_extensions<'c, H: HugrView>(
    cem: CodegenExtensionsBuilder<'c, H>,
    pcg: impl PreludeCodegen + 'c,
) -> CodegenExtensionsBuilder<'c, H> {
    let TypeEnum::Extension(qb_ext) = QB_T.as_type_enum().clone() else {
        unreachable!()
    };
    let TypeEnum::Extension(usize_ext) = USIZE_T.as_type_enum().clone() else {
        unreachable!()
    };
    let pcg_qb = pcg.clone();
    cem.add_type_by_key(
        (PRELUDE_ID,
        qb_ext.name().clone()),
        move |session, _| Ok(pcg_qb.qubit_type(session).as_basic_type_enum()))
    .add_type_by_key(
        (PRELUDE_ID,
        usize_ext.name().clone()),
        move |session, _| Ok(pcg.usize_type(session).as_basic_type_enum())
    )
    .add_custom_const::<ConstUsize>(|context, konst| {
        let ty: IntType<'c> = context
            .llvm_type(&konst.get_type())?
            .try_into()
            .map_err(|_| anyhow!("Failed to get ConstUsize as IntType"))?;
        Ok(ty.const_int(konst.value(), false).into())
    })
    .add_custom_const::<ConstExternalSymbol>(|context, konst| {
        let llvm_type = context.llvm_type(&konst.get_type())?;
        let global = context.get_global(&konst.symbol, llvm_type, konst.constant)?;
        Ok(context
            .builder()
            .build_load(global.as_pointer_value(), &konst.symbol)?)
    })
}

/// Add a [PreludeCodegenExtension] to the given [CodegenExtsMap] using
/// [DefaultPreludeCodegen] as the implementation.
pub fn add_default_prelude_extensions<H: HugrView>(cem: CodegenExtensionsBuilder<H>) -> CodegenExtensionsBuilder<H> {
    add_prelude_extensions(cem, DefaultPreludeCodegen)
}

impl<'c, H: HugrView> CodegenExtensionsBuilder<'c, H> {
    /// Add a [PreludeCodegenExtension] to the given [CodegenExtsMap] using `pcg`
    /// as the implementation.
    pub fn add_default_prelude_extensions(self) -> Self {
        add_default_prelude_extensions(self)
    }

    /// Add a [PreludeCodegenExtension] to the given [CodegenExtsMap] using
    /// [DefaultPreludeCodegen] as the implementation.
    pub fn add_prelude_extensions(self, builder: impl PreludeCodegen + 'c) -> Self {
        add_prelude_extensions(self, builder)
    }
}

#[cfg(test)]
mod test {
    use hugr::builder::{Dataflow, DataflowSubContainer};
    use hugr::extension::prelude::PRELUDE_REGISTRY;
    use hugr::type_row;
    use rstest::rstest;

    use crate::check_emission;
    use crate::emit::test::SimpleHugrConfig;
    use crate::test::{llvm_ctx, TestContext};
    use crate::types::HugrType;

    use super::*;

    #[derive(Clone)]
    struct TestPreludeCodegen;
    impl PreludeCodegen for TestPreludeCodegen {
        fn usize_type<'c, H>(&self, session: &TypingSession<'c, H>) -> IntType<'c> {
            session.iw_context().i32_type()
        }

        fn qubit_type<'c, H>(&self, session: &TypingSession<'c, H>) -> impl BasicType<'c> {
            session.iw_context().f64_type()
        }
    }

    #[rstest]
    fn prelude_extension_types_in_test_context(mut llvm_ctx: TestContext) {
        llvm_ctx.add_extensions(|x| x.add_prelude_extensions(TestPreludeCodegen));
        let tc = llvm_ctx.get_typing_session();
        assert_eq!(
            llvm_ctx.iw_context().i32_type().as_basic_type_enum(),
            tc.llvm_type(&USIZE_T).unwrap()
        );
        assert_eq!(
            llvm_ctx.iw_context().f64_type().as_basic_type_enum(),
            tc.llvm_type(&QB_T).unwrap()
        );
    }

    #[rstest]
    fn prelude_const_usize(mut llvm_ctx: TestContext) {
        llvm_ctx.add_extensions(add_default_prelude_extensions);

        let hugr = SimpleHugrConfig::new()
            .with_outs(USIZE_T)
            .with_extensions(PRELUDE_REGISTRY.to_owned())
            .finish(|mut builder| {
                let k = builder.add_load_value(ConstUsize::new(17));
                builder.finish_with_outputs([k]).unwrap()
            });
        check_emission!(hugr, llvm_ctx);
    }

    #[rstest]
    fn prelude_const_external_symbol(mut llvm_ctx: TestContext) {
        llvm_ctx.add_extensions(add_default_prelude_extensions);
        let konst1 = ConstExternalSymbol::new("sym1", USIZE_T, true);
        let konst2 = ConstExternalSymbol::new(
            "sym2",
            HugrType::new_sum([type_row![USIZE_T, HugrType::new_unit_sum(3)], type_row![]]),
            false,
        );

        let hugr = SimpleHugrConfig::new()
            .with_outs(vec![konst1.get_type(), konst2.get_type()])
            .with_extensions(PRELUDE_REGISTRY.to_owned())
            .finish(|mut builder| {
                let k1 = builder.add_load_value(konst1);
                let k2 = builder.add_load_value(konst2);
                builder.finish_with_outputs([k1, k2]).unwrap()
            });
        check_emission!(hugr, llvm_ctx);
    }
}
