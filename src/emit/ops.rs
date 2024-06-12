use std::collections::HashMap;

use anyhow::{anyhow, ensure, Result};
use hugr::{
    hugr::views::SiblingGraph,
    ops::{
        Call, Case, Conditional, Const, DataflowBlock, ExitBlock, Input, LoadConstant, MakeTuple,
        NamedOp, OpTag, OpTrait, OpType, Output, Tag, UnpackTuple, Value, CFG,
    },
    types::{SumType, Type, TypeEnum},
    HugrView, NodeIndex,
};
use inkwell::{
    basic_block::BasicBlock, builder::Builder, types::BasicType, values::BasicValueEnum,
};
use itertools::Itertools;
use petgraph::visit::Walker;

use crate::fat::FatExt as _;
use crate::{fat::FatNode, types::LLVMSumType};

use super::{
    func::{EmitFuncContext, RowMailBox, RowPromise},
    EmitOp, EmitOpArgs,
};

struct SumOpEmitter<'c, 'd, H: HugrView>(&'d mut EmitFuncContext<'c, H>, LLVMSumType<'c>);

impl<'c, 'd, H: HugrView> SumOpEmitter<'c, 'd, H> {
    pub fn new(context: &'d mut EmitFuncContext<'c, H>, st: LLVMSumType<'c>) -> Self {
        Self(context, st)
    }
    pub fn try_new(
        context: &'d mut EmitFuncContext<'c, H>,
        ts: impl IntoIterator<Item = Type>,
    ) -> Result<Self> {
        let llvm_sum_type = context.llvm_sum_type(get_exactly_one_sum_type(ts)?)?;
        Ok(Self::new(context, llvm_sum_type))
    }
}

impl<'c, H: HugrView> EmitOp<'c, MakeTuple, H> for SumOpEmitter<'c, '_, H> {
    fn emit(&mut self, args: EmitOpArgs<'c, MakeTuple, H>) -> Result<()> {
        let builder = self.0.builder();
        println!("dougrulz3");
        let r = args
            .outputs
            .finish(builder, [self.1.build_tag(builder, 0, args.inputs)?])?;
        println!("dougrulz4");
        Ok(r)
    }
}

impl<'c, H: HugrView> EmitOp<'c, UnpackTuple, H> for SumOpEmitter<'c, '_, H> {
    fn emit(&mut self, args: EmitOpArgs<'c, UnpackTuple, H>) -> Result<()> {
        let builder = self.0.builder();
        let input = args
            .inputs
            .into_iter()
            .exactly_one()
            .map_err(|_| anyhow!("unpacktuple expected exactly one input"))?;
        args.outputs
            .finish(builder, self.1.build_untag(builder, 0, input)?)
    }
}

impl<'c, H: HugrView> EmitOp<'c, Tag, H> for SumOpEmitter<'c, '_, H> {
    fn emit(&mut self, args: EmitOpArgs<'c, Tag, H>) -> Result<()> {
        println!("dougrulz5");
        let builder = self.0.builder();
        let r = args.outputs.finish(
            builder,
            [self
                .1
                .build_tag(builder, args.node.tag as u32, args.inputs)?],
        )?;
        println!("dougrulz6");
        Ok(r)
    }
}

struct DataflowParentEmitter<'c, 'd, OT, H: HugrView> {
    context: &'d mut EmitFuncContext<'c, H>,
    node: FatNode<'c, OT, H>,
    inputs: Option<Vec<BasicValueEnum<'c>>>,
    outputs: Option<RowPromise<'c>>,
}

impl<'c, 'd, OT: OpTrait + 'c, H: HugrView> DataflowParentEmitter<'c, 'd, OT, H>
where
    &'c OpType: TryInto<&'c OT>,
    // &'c OpType: TryInto<&'c OT>,
    // <&'c OpType as TryInto<&'c OT>>::Error: std::fmt::Debug,
{
    pub fn new(context: &'d mut EmitFuncContext<'c, H>, args: EmitOpArgs<'c, OT, H>) -> Self {
        Self {
            context,
            node: args.node,
            inputs: Some(args.inputs),
            outputs: Some(args.outputs),
        }
    }

    /// safe because we are guarenteed only one input or output node
    fn take_input(&mut self) -> Result<Vec<BasicValueEnum<'c>>> {
        self.inputs
            .take()
            .ok_or(anyhow!("DataflowParentEmitter: Input taken twice"))
    }

    fn take_output(&mut self) -> Result<RowPromise<'c>> {
        self.outputs
            .take()
            .ok_or(anyhow!("DataflowParentEmitter: Output taken twice"))
    }

    pub fn builder(&mut self) -> &Builder<'c> {
        self.context.builder()
    }

    pub fn emit_children(mut self) -> Result<()> {
        use hugr::hugr::views::HierarchyView;
        use petgraph::visit::Topo;
        let node = self.node.clone();
        if !OpTag::DataflowParent.is_superset(OpTrait::tag(node.get())) {
            Err(anyhow!("Not a dataflow parent"))?
        };

        let (i, o): (FatNode<Input, H>, FatNode<Output, H>) = node
            .get_io()
            .ok_or(anyhow!("emit_dataflow_parent: no io nodes"))?;
        debug_assert!(i.out_value_types().count() == self.inputs.as_ref().unwrap().len());
        debug_assert!(o.in_value_types().count() == self.outputs.as_ref().unwrap().len());

        let region: SiblingGraph = SiblingGraph::try_new(node.hugr(), node.node()).unwrap();
        Topo::new(&region.as_petgraph())
            .iter(&region.as_petgraph())
            .filter(|x| (*x != node.node()))
            .map(|x| node.hugr().fat_optype(x))
            .try_for_each(|node| {
                let inputs_rmb = self.context.node_ins_rmb(node.clone())?;
                let inputs = inputs_rmb.read(self.builder(), [])?;
                let outputs = self.context.node_outs_rmb(node.clone())?.promise();
                self.emit(EmitOpArgs {
                    node,
                    inputs,
                    outputs,
                })
            })
    }
}

impl<'c, OT: OpTrait + 'c, H: HugrView> EmitOp<'c, OpType, H>
    for DataflowParentEmitter<'c, '_, OT, H>
where
    &'c OpType: TryInto<&'c OT>,
{
    fn emit(&mut self, args: EmitOpArgs<'c, OpType, H>) -> Result<()> {
        if !OpTag::DataflowChild.is_superset(args.node().tag()) {
            Err(anyhow!("Not a dataflow child"))?
        };

        match args.node().get() {
            OpType::Input(_) => {
                let i = self.take_input()?;
                args.outputs.finish(self.builder(), i)
            }
            OpType::Output(_) => {
                let o = self.take_output()?;
                o.finish(self.builder(), args.inputs)
            }
            _ => emit_optype(self.context, args),
        }
    }
}

struct ConditionalEmitter<'c, 'd, H: HugrView>(&'d mut EmitFuncContext<'c, H>);

impl<'c, H: HugrView> EmitOp<'c, Conditional, H> for ConditionalEmitter<'c, '_, H> {
    fn emit(
        &mut self,
        EmitOpArgs {
            node,
            inputs,
            outputs,
        }: EmitOpArgs<'c, Conditional, H>,
    ) -> Result<()> {
        let context = &mut self.0;
        let exit_rmb = context
            .new_row_mail_box(node.dataflow_signature().unwrap().output.iter(), "exit_rmb")?;
        let exit_block = context.build_positioned_new_block(
            format!("cond_exit_{}", node.node().index()),
            None,
            |context, bb| {
                let builder = context.builder();
                outputs.finish(builder, exit_rmb.read_vec(builder, [])?)?;
                Ok::<_, anyhow::Error>(bb)
            },
        )?;

        let case_values_rmbs_blocks = node
            .children()
            .enumerate()
            .map(|(i, n)| {
                let label = format!("cond_{}_case_{}", node.node().index(), i);
                let node = n.try_into_ot::<Case>().ok_or(anyhow!("not a case node"))?;
                let rmb =
                    context.new_row_mail_box(node.get_io().unwrap().0.types.iter(), &label)?;
                context.build_positioned_new_block(&label, Some(exit_block), |context, bb| {
                    let inputs = rmb.read_vec(context.builder(), [])?;
                    emit_dataflow_parent(
                        context,
                        EmitOpArgs {
                            node,
                            inputs,
                            outputs: exit_rmb.promise(),
                        },
                    )?;
                    context.builder().build_unconditional_branch(exit_block)?;
                    Ok((i, rmb, bb))
                })
            })
            .collect::<Result<Vec<_>>>()?;

        let sum_type = get_exactly_one_sum_type(node.in_value_types().next().map(|x| x.1))?;
        let llvm_sum_type = context.llvm_sum_type(sum_type)?;
        debug_assert!(inputs[0].get_type() == llvm_sum_type.as_basic_type_enum());

        let sum_input = inputs[0].into_struct_value();
        let builder = context.builder();
        let tag = llvm_sum_type.build_get_tag(builder, sum_input)?;
        let switches = case_values_rmbs_blocks
            .into_iter()
            .map(|(i, rmb, bb)| {
                let mut vs = llvm_sum_type.build_untag(builder, i as u32, sum_input)?;
                vs.extend(&inputs[1..]);
                rmb.write(builder, vs)?;
                Ok((llvm_sum_type.get_tag_type().const_int(i as u64, false), bb))
            })
            .collect::<Result<Vec<_>>>()?;

        builder.build_switch(tag, switches[0].1, &switches[1..])?;
        builder.position_at_end(exit_block);
        Ok(())
    }
}

struct CfgEmitter<'c, 'd, H: HugrView> {
    context: &'d mut EmitFuncContext<'c, H>,
    bbs: HashMap<FatNode<'c, OpType, H>, (BasicBlock<'c>, RowMailBox<'c>)>,
    inputs: Option<Vec<BasicValueEnum<'c>>>,
    outputs: Option<RowPromise<'c>>,
    node: FatNode<'c, CFG, H>,
    entry_node: FatNode<'c, DataflowBlock, H>,
    exit_node: FatNode<'c, ExitBlock, H>,
}

impl<'c, 'd, H: HugrView> CfgEmitter<'c, 'd, H> {
    pub fn new(
        context: &'d mut EmitFuncContext<'c, H>,
        args: EmitOpArgs<'c, CFG, H>,
    ) -> Result<Self> {
        let node = args.node();
        let (inputs, outputs) = (Some(args.inputs), Some(args.outputs));
        let out_types = node.out_value_types().map(|x| x.1).collect_vec();
        let output_row = context.new_row_mail_box(out_types.iter(), "")?;
        let exit_block = context.new_basic_block("", None);
        let bbs = node
            .children()
            .map(|child| {
                if child.is_exit_block() {
                    Ok((child, (exit_block, output_row.clone())))
                } else {
                    let bb = context.new_basic_block("", Some(exit_block));
                    let (i, _) = child.get_io().unwrap();
                    Ok((child, (bb, context.node_outs_rmb(i)?)))
                }
            })
            .collect::<Result<HashMap<_, _>>>()?;
        let [entry_node, exit_node] = node
            .children()
            .take(2)
            .collect_vec()
            .try_into()
            .map_err(|_| anyhow!("cfg doesn't have two children"))?;
        let entry_node = entry_node.try_into_ot::<DataflowBlock>().unwrap();
        let exit_node = exit_node.try_into_ot::<ExitBlock>().unwrap();
        Ok(CfgEmitter {
            context,
            bbs,
            node,
            inputs,
            outputs,
            entry_node,
            exit_node,
        })
    }

    fn take_inputs(&mut self) -> Result<Vec<BasicValueEnum<'c>>> {
        self.inputs.take().ok_or(anyhow!("Couldn't take inputs"))
    }

    fn take_outputs(&mut self) -> Result<RowPromise<'c>> {
        self.outputs.take().ok_or(anyhow!("Couldn't take inputs"))
    }

    fn get_block_data<OT>(
        &self,
        node: &FatNode<'c, OT, H>,
    ) -> Result<&(BasicBlock<'c>, RowMailBox<'c>)>
    where
        OT: Into<OpType> + 'c,
    {
        self.bbs
            .get(&node.clone().generalise())
            .ok_or(anyhow!("Couldn't get block data for: {}", node.index()))
    }

    fn emit_children(mut self) -> Result<()> {
        let inputs = self.take_inputs()?;
        let (entry_bb, inputs_rmb) = self.get_block_data(&self.entry_node).cloned()?;
        let builder = self.context.builder();
        inputs_rmb.write(builder, inputs)?;
        builder.build_unconditional_branch(entry_bb)?;

        for c in self.node.children() {
            let (inputs, outputs) = (vec![], RowMailBox::new_empty().promise());
            if let Some(node) = c.try_into_ot::<DataflowBlock>() {
                self.emit(EmitOpArgs {
                    node,
                    inputs,
                    outputs,
                })?;
            } else if let Some(node) = c.try_into_ot::<ExitBlock>() {
                self.emit(EmitOpArgs {
                    node,
                    inputs,
                    outputs,
                })?;
            } else {
                Err(anyhow!("unknown optype: {c}"))?;
            }
        }
        let outputs = self.take_outputs()?;
        let (exit_bb, outputs_rmb) = self.get_block_data(&self.exit_node).cloned()?;
        let builder = self.context.builder();
        builder.position_at_end(exit_bb);
        outputs.finish(builder, outputs_rmb.read_vec(builder, [])?)?;
        Ok(())
    }
}

impl<'c, H: HugrView> EmitOp<'c, DataflowBlock, H> for CfgEmitter<'c, '_, H> {
    fn emit(
        &mut self,
        EmitOpArgs {
            node,
            inputs: _,
            outputs: _,
        }: EmitOpArgs<'c, DataflowBlock, H>,
    ) -> Result<()> {
        let (bb, inputs_rmb) = self.bbs.get(&node.clone().generalise()).unwrap();
        let (_, o) = node.get_io().unwrap();
        let successor_data = node
            .output_neighbours()
            .map(|succ| self.get_block_data(&succ).map(|x| x.clone()))
            .collect::<Result<Vec<_>>>()?;

        self.context.build_positioned(*bb, |context| {
            let outputs_rmb = context.node_ins_rmb(o)?;
            let inputs = inputs_rmb.read_vec(context.builder(), [])?;
            emit_dataflow_parent(
                context,
                EmitOpArgs {
                    node: node.clone(),
                    inputs,
                    outputs: outputs_rmb.promise(),
                },
            )?;

            let outputs = outputs_rmb.read_vec(context.builder(), [])?;
            let branch_sum_type = SumType::new(node.sum_rows.clone());
            let llvm_sum_type = context.llvm_sum_type(branch_sum_type)?;
            let tag_bbs = successor_data
                .into_iter()
                .enumerate()
                .map(|(tag, (target_bb, target_rmb))| {
                    let bb = context.build_positioned_new_block("", Some(*bb), |context, bb| {
                        let builder = context.builder();
                        let mut vals =
                            llvm_sum_type.build_untag(builder, tag as u32, outputs[0])?;
                        vals.extend(&outputs[1..]);
                        target_rmb.write(builder, vals)?;
                        builder.build_unconditional_branch(target_bb)?;
                        Ok::<_, anyhow::Error>(bb)
                    })?;
                    Ok((
                        llvm_sum_type.get_tag_type().const_int(tag as u64, false),
                        bb,
                    ))
                })
                .collect::<Result<Vec<_>>>()?;
            let tag_v = llvm_sum_type.build_get_tag(context.builder(), outputs[0])?;
            context
                .builder()
                .build_switch(tag_v, tag_bbs[0].1, &tag_bbs[1..])?;
            Ok(())
        })
    }
}

impl<'c, H: HugrView> EmitOp<'c, ExitBlock, H> for CfgEmitter<'c, '_, H> {
    fn emit(
        &mut self,
        EmitOpArgs {
            node,
            inputs: _,
            outputs: _,
        }: EmitOpArgs<'c, ExitBlock, H>,
    ) -> Result<()> {
        Ok(())
    }
}

fn emit_cfg<'c, H: HugrView>(
    context: &mut EmitFuncContext<'c, H>,
    args: EmitOpArgs<'c, CFG, H>,
) -> Result<()> {
    CfgEmitter::new(context, args)?.emit_children()
}

fn get_exactly_one_sum_type(ts: impl IntoIterator<Item = Type>) -> Result<SumType> {
    let Some(TypeEnum::Sum(sum_type)) = ts
        .into_iter()
        .map(|t| t.as_type_enum().clone())
        .exactly_one()
        .ok()
    else {
        Err(anyhow!("Not exactly one SumType"))?
    };
    Ok(sum_type)
}

fn emit_value<'c, H: HugrView>(
    context: &mut EmitFuncContext<'c, H>,
    v: &Value,
) -> Result<BasicValueEnum<'c>> {
    println!("emit_value: {v:?}");
    match v {
        Value::Extension { e } => {
            let exts = context.extensions();
            let val = exts.load_constant(context, e.value())?;
            ensure!(val.get_type() == context.llvm_type(&v.get_type())?);
            Ok(val)
        }
        Value::Function { .. } => todo!(),
        Value::Tuple { vs } => {
            println!("dougrulz1");
            let tys = vs.iter().map(|x| x.get_type()).collect_vec();
            let llvm_st = LLVMSumType::try_new(&context.typing_session(), SumType::new([tys]))?;
            let llvm_vs = vs
                .iter()
                .map(|x| emit_value(context, x))
                .collect::<Result<Vec<_>>>()?;
            let r = llvm_st.build_tag(context.builder(), 0, llvm_vs)?;
            println!("dougrulz2");
            Ok(r)
        }
        Value::Sum {
            tag,
            values,
            sum_type,
        } => {
            println!("dougrulz7");
            let llvm_st = LLVMSumType::try_new(&context.typing_session(), sum_type.clone())?;
            let vs = values
                .iter()
                .map(|x| emit_value(context, x))
                .collect::<Result<Vec<_>>>()?;
            let r = llvm_st.build_tag(context.builder(), *tag as u32, vs)?;
            println!("dougrulz8");
            Ok(r)
        }
    }
}

pub(crate) fn emit_dataflow_parent<'c, OT: OpTrait + 'c, H: HugrView>(
    context: &mut EmitFuncContext<'c, H>,
    args: EmitOpArgs<'c, OT, H>,
) -> Result<()>
where
    &'c OpType: TryInto<&'c OT>,
{
    DataflowParentEmitter::new(context, args).emit_children()
}

fn emit_make_tuple<'c, H: HugrView>(
    context: &mut EmitFuncContext<'c, H>,
    args: EmitOpArgs<'c, MakeTuple, H>,
) -> Result<()> {
    SumOpEmitter::try_new(context, args.node.out_value_types().map(|x| x.1))?.emit(args)
}

fn emit_unpack_tuple<'c, H: HugrView>(
    context: &mut EmitFuncContext<'c, H>,
    args: EmitOpArgs<'c, UnpackTuple, H>,
) -> Result<()> {
    SumOpEmitter::try_new(context, args.node.in_value_types().map(|x| x.1))?.emit(args)
}

fn emit_tag<'c, H: HugrView>(
    context: &mut EmitFuncContext<'c, H>,
    args: EmitOpArgs<'c, Tag, H>,
) -> Result<()> {
    SumOpEmitter::try_new(context, args.node.out_value_types().map(|x| x.1))?.emit(args)
}

fn emit_conditional<'c, H: HugrView>(
    context: &mut EmitFuncContext<'c, H>,
    args: EmitOpArgs<'c, Conditional, H>,
) -> Result<()> {
    ConditionalEmitter(context).emit(args)
}

fn emit_load_constant<'c, H: HugrView>(
    context: &mut EmitFuncContext<'c, H>,
    args: EmitOpArgs<'c, LoadConstant, H>,
) -> Result<()> {
    let konst_node = args
        .node
        .single_linked_output(0.into())
        .unwrap()
        .0
        .try_into_ot::<Const>()
        .unwrap();
    let r = emit_value(context, konst_node.value())?;
    args.outputs.finish(context.builder(), [r])
}

fn emit_call<'c, H: HugrView>(
    context: &mut EmitFuncContext<'c, H>,
    args: EmitOpArgs<'c, Call, H>,
) -> Result<()> {
    if !args.node.called_function_type().params().is_empty() {
        todo!("Call of generic function");
    }
    let (func_node, _) = args
        .node
        .single_linked_output(args.node.called_function_port())
        .unwrap();
    let func = match func_node.get() {
        OpType::FuncDecl(_) => context.get_func_decl(func_node.try_into_ot().unwrap()),
        OpType::FuncDefn(_) => context.get_func_defn(func_node.try_into_ot().unwrap()),
        _ => Err(anyhow!("emit_call: Not a Decl or Defn")),
    };
    let inputs = args.inputs.into_iter().map_into().collect_vec();
    let builder = context.builder();
    let call = builder
        .build_call(func?, inputs.as_slice(), "")?
        .try_as_basic_value();
    let rets = match args.outputs.len() as u32 {
        0 => {
            call.expect_right("void");
            vec![]
        }
        1 => vec![call.expect_left("non-void")],
        n => {
            let return_struct = call.expect_left("non-void").into_struct_value();
            (0..n)
                .map(|i| builder.build_extract_value(return_struct, i, ""))
                .collect::<Result<Vec<_>, _>>()?
        }
    };
    args.outputs.finish(builder, rets)
}

fn emit_optype<'c, H: HugrView>(
    context: &mut EmitFuncContext<'c, H>,
    args: EmitOpArgs<'c, OpType, H>,
) -> Result<()> {
    let node = args.node();
    match node.get() {
        OpType::MakeTuple(ref mt) => emit_make_tuple(context, args.into_ot(mt)),
        OpType::UnpackTuple(ref ut) => emit_unpack_tuple(context, args.into_ot(ut)),
        OpType::Tag(ref tag) => emit_tag(context, args.into_ot(tag)),
        OpType::DFG(_) => emit_dataflow_parent(context, args),

        OpType::CustomOp(ref co) => {
            let extensions = context.extensions();
            extensions.emit(context, args.into_ot(co))
        }
        OpType::Const(_) => Ok(()),
        OpType::LoadConstant(ref lc) => emit_load_constant(context, args.into_ot(lc)),
        OpType::Call(ref cl) => emit_call(context, args.into_ot(cl)),
        OpType::Conditional(ref co) => emit_conditional(context, args.into_ot(co)),
        OpType::CFG(ref cfg) => emit_cfg(context, args.into_ot(cfg)),

        // OpType::FuncDefn(fd) => self.emit(ot.into_ot(fd), context, inputs, outputs),
        _ => todo!("Unimplemented OpTypeEmitter: {}", args.node().name()),
    }
}
