use std::{
    any::{type_name_of_val, TypeId},
    collections::HashMap,
    rc::Rc,
};

use hugr::{
    extension::{simple_op::MakeOpDef, ExtensionId},
    ops::{constant::CustomConst, CustomOp, NamedOp, OpType},
    types::{CustomType, TypeName},
    HugrView, OutgoingPort,
};

use strum::IntoEnumIterator;

use anyhow::{anyhow, Result};
use inkwell::{
    builder::Builder, types::BasicTypeEnum, values::{BasicValueEnum, PointerValue}
};

use crate::{
    emit::{func::EmitFuncContext, EmitOpArgs}, fat::FatNode, type_map::{def_hook::{DefHookTypeMap, DefHookTypeMapping}, destructor::{Destructor, DestructorTypeMapping}, CustomTypeKey, TypeMapping}, types::TypingSession
};

use super::emit::EmitOp;

pub mod float;
pub mod int;
pub mod prelude;
pub mod ptr;


pub trait LoadConstHandler<'c, H, CC: ?Sized>:
    Fn(&mut EmitFuncContext<'c, H>, &CC) -> Result<BasicValueEnum<'c>> + 'c
{
    fn load_constant(
        &self,
        context: &mut EmitFuncContext<'c, H>,
        konst: &CC,
    ) -> Result<BasicValueEnum<'c>>;
}

impl<
        'c,
        H,
        CC: CustomConst + ?Sized,
        F: Fn(&mut EmitFuncContext<'c, H>, &CC) -> Result<BasicValueEnum<'c>> + 'c,
    > LoadConstHandler<'c, H, CC> for F
{
    fn load_constant(
        &self,
        context: &mut EmitFuncContext<'c, H>,
        konst: &CC,
    ) -> Result<BasicValueEnum<'c>> {
        self(context, konst)
    }
}

pub trait OpHandler<'c, H>: 'c {
    fn emitter<'a>(
        &self,
        context: &'a mut EmitFuncContext<'c, H>,
    ) -> Box<dyn EmitOp<'c, CustomOp, H> + 'a>;
}

impl<'c, H, F> OpHandler<'c, H> for F
where
    F: for<'a> Fn(&'a mut EmitFuncContext<'c, H>) -> Box<dyn EmitOp<'c, CustomOp, H> + 'a> + 'c,
{
    fn emitter<'a>(
        &self,
        context: &'a mut EmitFuncContext<'c, H>,
    ) -> Box<dyn EmitOp<'c, CustomOp, H> + 'a> {
        self(context)
    }
}

// pub trait CompositeTypeHandler<'c, H>: 'c {
//     fn llvm_type(
//         &self,
//         context: &TypingSession<'c, H>,
//         typ: &CustomType,
//     ) -> Result<SimpleHookedType<'c, H>>;
// }

// impl<'c, H, F> CompositeTypeHandler<'c, H> for F
// where
//     F: Fn(&TypingSession<'c, H>, &CustomType) -> Result<SimpleHookedType<'c, H>> + 'c,
// {
//     fn llvm_type(
//         &self,
//         context: &TypingSession<'c, H>,
//         typ: &CustomType,
//     ) -> Result<SimpleHookedType<'c, H>> {
//         self(context, typ)
//     }
// }

pub trait TypeHandler<'c, H> : Fn(&TypingSession<'c,H>, &CustomType) -> Result<BasicTypeEnum<'c>> {}
impl<'c, H, F: Fn(&TypingSession<'c,H>, &CustomType) -> Result<BasicTypeEnum<'c>> + ?Sized> TypeHandler<'c,H> for F{}


#[derive(Default)]
pub struct CodegenExtensionsBuilder<'c, H> {
    ops: HashMap<String, Rc<dyn OpHandler<'c, H>>>,
    types: HashMap<CustomTypeKey, Box<dyn TypeHandler<'c,H> + 'c>>,
    consts: HashMap<TypeId, Rc<dyn LoadConstHandler<'c, H, dyn CustomConst>>>,
    def_hooks: DefHookTypeMap<'c,'c,H>,
    destructors: Destructor<'c,'c,H>,
}


pub type CodegenExtsMap<'c,H> = CodegenExtensionsBuilder<'c,H>;

impl<'c, H> CodegenExtensionsBuilder<'c, H> {
    pub fn add_custom_const<CC: CustomConst>(
        mut self,
        handler: impl LoadConstHandler<'c, H, CC>,
    ) -> Self {
        self.consts.insert(
            TypeId::of::<CC>(),
            Rc::new(move |context, konst| {
                let Some(konst) = konst.downcast_ref::<CC>() else {
                    Err(anyhow!("bad konst"))?
                };
                handler(context, konst)
            }),
        );
        self
    }

    pub fn add_simple_op<OP: MakeOpDef + IntoEnumIterator>(
        mut self,
        handler: impl OpHandler<'c, H>,
    ) -> Self {
        let handler: Rc<dyn OpHandler<'c, H>> = Rc::new(handler);
        self.ops.extend(
            OP::iter().map(|op| (format!("{}.{}", op.extension(), op.name()), handler.clone())),
        );
        self
    }

    pub fn add_op(
        mut self,
        ext: &ExtensionId,
        op: impl AsRef<str>,
        handler: impl OpHandler<'c, H>,
    ) -> Self {
        self.ops
            .insert(format!("{ext}.{}", op.as_ref()), Rc::new(handler));
        self
    }

    pub fn add_type(
        self,
        typ: &CustomType,
        handler: impl TypeHandler<'c,H> + 'c
    ) -> Self {
        self.add_type_by_key((typ.extension().to_owned(), typ.name().to_owned()), handler)
    }

    pub fn add_type_by_key(
        mut self,
        key: CustomTypeKey,
        handler: impl TypeHandler<'c,H> + 'c
    ) -> Self {
        self.types.insert(key, Box::new(handler));
        self
    }

    pub fn merge(mut self, other: Self) -> Self {
        self.ops.extend(other.ops);
        self.types.extend(other.types);
        self.consts.extend(other.consts);
        self
    }

    pub fn set_def_hook_by_key(
        mut self,
        key: CustomTypeKey,
        hook: impl TypeMapping<'c, DefHookTypeMapping<'c, 'c, H>> + 'c,
    ) -> Self {
        self.def_hooks.set_def_hook(key, hook);
        self
    }

    pub fn set_destructor(
        mut self,
        key: CustomTypeKey,
        hook: impl TypeMapping<'c, DestructorTypeMapping<'c, 'c, H>> + 'c,
    ) -> Self {
        self.destructors.set_destructor(key, hook);
        self
    }

    pub fn llvm_type(
        &self,
        session: &TypingSession<'c, H>,
        typ: &CustomType,
    ) -> Result<BasicTypeEnum<'c>> {
        let handler = self.types.get(&(typ.extension().to_owned(), typ.name().to_owned())).ok_or(anyhow!("No handler for type: {typ}"))?;
        handler(session,typ)
    }

    pub fn emit_load_constant(
        &self,
        context: &mut EmitFuncContext<'c, H>,
        konst: &dyn CustomConst,
    ) -> Result<BasicValueEnum<'c>> {
        let handler = self
            .consts
            .get(&konst.type_id())
            .ok_or(anyhow!("Unknown CustomConst: {}", type_name_of_val(konst)))?;
        handler.load_constant(context, konst)
    }

    pub fn emitter<'a>(
        &self,
        context: &'a mut EmitFuncContext<'c, H>,
        op: &CustomOp,
    ) -> Result<Box<dyn EmitOp<'c, CustomOp, H> + 'a>> {
        let handler = self
            .ops
            .get(&op.name().to_string())
            .ok_or(anyhow!("Unknown op: {}", op.name()))?;
        Ok(handler.emitter(context))
    }

    pub fn emit(
        &self,
        context: &mut EmitFuncContext<'c, H>,
        args: EmitOpArgs<'c, CustomOp, H>,
    ) -> Result<()>
    where
        H: HugrView,
    {
        let mut emitter = self.emitter(context, args.node().as_ref())?;
        emitter.emit(args)
    }

    // fn lookup_type(&self, typ: &CustomType) -> Result<&TypeReg<'c, H>> {
    //     self.types
    //         .get(&(typ.extension().to_owned(), typ.name().to_owned()))
    //         .ok_or(anyhow!("Unknown typ: {}", typ.name()))
    // }

    // pub fn emit_destructor(
    //     &self,
    //     context: &mut EmitFuncContext<'c, H>,
    //     typ: &CustomType,
    //     value: PointerValue<'c>,
    // ) -> Result<()> {
    //     match self.lookup_type(typ)? {
    //         TypeReg::Composite(_) => todo!(),
    //         TypeReg::Leaf { destructor, .. } => destructor
    //             .as_ref()
    //             .map_or(Ok(()), |d| d.emit_destructor(context, value)),
    //     }
    // }

    // pub fn emit_def_hook(
    //     &self,
    //     context: &mut EmitFuncContext<'c, H>,
    //     typ: &CustomType,
    //     value: BasicValueEnum<'c>,
    //     node: FatNode<'c, OpType, H>,
    //     port: OutgoingPort,
    // ) -> Result<BasicValueEnum<'c>> {
    //     match self.lookup_type(typ)? {
    //         TypeReg::Composite(_) => todo!(),
    //         TypeReg::Leaf { def_hook, .. } => def_hook
    //             .as_ref()
    //             .map_or(Ok(value), |d| d.emit_def_hook(context, value, node, port)),
    //     }
    // }
}

// pub fn is_simple_type(&self, typ: HugrType) -> Result<bool> {
//     match typ.as_type_enum() {
//         hugr::types::TypeEnum::Extension(ct) => Ok(match self.lookup_type(ct)? {
//             TypeReg::Leaf{..} => true,
//             _ => false
//         }),
//         hugr::types::TypeEnum::Function(_) => Ok(true),
//         hugr::types::TypeEnum::Sum(st) => {
//                                                                     TypeRow::try_from(st.get_variant(i).unwrap().to_owned()).map_err(|rv| anyhow!("Row variable: {rv}"))).collect::<Result<Vec<_>>>()?;
//             Ok(tys.into_iter().flat_map(|tr| tr.iter()).all(|x| self.is_simple_type(x)))
//         }
//         x => Err(anyhow!("Invalid type: {:?}", x)),
//     }

// }

/// A collection of [CodegenExtension]s.
///
/// Provides methods to delegate operations to appropriate contained
/// [CodegenExtension]s.
// #[derive(Clone,Default)]
// pub struct CodegenExtsMap<'c, H>(CGReg<'c, H>);

// impl<'c,H> From<CGReg<'c,H>> for CodegenExtsMap<'c,H> {
//     fn from(value: CGReg<'c,H>) -> Self {
//         Self(value)
//     }
// }

// TODO upstream this to hugr
fn custom_op_extension(o: &CustomOp) -> &ExtensionId {
    match o {
        CustomOp::Extension(e) => e.def().extension(),
        CustomOp::Opaque(o) => o.extension(),
    }
}

