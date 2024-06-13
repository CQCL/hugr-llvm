use std::collections::HashMap;

use anyhow::{anyhow, Result};
use hugr::{
    ops::{DataflowBlock, ExitBlock, OpType, CFG},
    types::SumType,
    HugrView, NodeIndex,
};
use inkwell::{basic_block::BasicBlock, values::BasicValueEnum};
use itertools::Itertools as _;

use crate::{
    emit::{
        func::{EmitFuncContext, RowMailBox, RowPromise},
        EmitOp, EmitOpArgs,
    },
    fat::FatNode,
};

use super::emit_dataflow_parent;

pub struct CfgEmitter<'c, 'd, H: HugrView> {
    context: &'d mut EmitFuncContext<'c, H>,
    bbs: HashMap<FatNode<'c, OpType, H>, (BasicBlock<'c>, RowMailBox<'c>)>,
    inputs: Option<Vec<BasicValueEnum<'c>>>,
    outputs: Option<RowPromise<'c>>,
    node: FatNode<'c, CFG, H>,
    entry_node: FatNode<'c, DataflowBlock, H>,
    exit_node: FatNode<'c, ExitBlock, H>,
}

impl<'c, 'd, H: HugrView> CfgEmitter<'c, 'd, H> {
    // Constructs a new CfgEmitter. Creates a basic block for each of
    // the children in the llvm function. Note that this does not move the
    // position of the builder.
    pub fn new(
        context: &'d mut EmitFuncContext<'c, H>,
        args: EmitOpArgs<'c, CFG, H>,
    ) -> Result<Self> {
        let node = args.node();
        let (inputs, outputs) = (Some(args.inputs), Some(args.outputs));

        // create this now so that it will be the last block and we can use it
        // to crate the other blocks immediately before it. This is just for
        // nice block ordering.
        let exit_block = context.new_basic_block("", None);
        let bbs = node
            .children()
            .map(|child| {
                if child.is_exit_block() {
                    let output_row = {
                        let out_types = node.out_value_types().map(|x| x.1).collect_vec();
                        context.new_row_mail_box(out_types.iter(), "")?
                    };
                    Ok((child, (exit_block, output_row)))
                } else {
                    let bb = context.new_basic_block("", Some(exit_block));
                    let (i, _) = child.get_io().unwrap();
                    Ok((child, (bb, context.node_outs_rmb(i)?)))
                }
            })
            .collect::<Result<HashMap<_, _>>>()?;
        let (entry_node, exit_node) = node.get_entry_exit().unwrap();
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

    /// Consume the emitter by emitting each child of the node.
    /// After returning the builder will be at the end of the exit block.
    pub fn emit_children(mut self) -> Result<()> {
        // write the inputs of the cfg node into the inputs of the entry
        // dataflowblock node, and then branch to the basic block of that entry
        // node.
        let inputs = self.take_inputs()?;
        let (entry_bb, inputs_rmb) = self.get_block_data(&self.entry_node).cloned()?;
        let builder = self.context.builder();
        inputs_rmb.write(builder, inputs)?;
        builder.build_unconditional_branch(entry_bb)?;

        // emit each child by delegating to the `impl EmitOp<_>` of self.
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

        // move the builder to the end of the exit block
        let (exit_bb, _) = self.get_block_data(&self.exit_node).cloned()?;
        self.context.builder().position_at_end(exit_bb);
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
        // our entry basic block and our input RowMailBox
        let (bb, inputs_rmb) = self.bbs.get(&node.clone().generalise()).unwrap();
        // the basic block and mailbox of each of our successors
        let successor_data = node
            .output_neighbours()
            .map(|succ| self.get_block_data(&succ).map(|x| x.clone()))
            .collect::<Result<Vec<_>>>()?;

        self.context.build_positioned(*bb, |context| {
            let (_, o) = node.get_io().unwrap();
            // get the rowmailbox for our output node
            let outputs_rmb = context.node_ins_rmb(o)?;
            // read the values from our input node
            let inputs = inputs_rmb.read_vec(context.builder(), [])?;

            // emit all our children and read the values from the rowmailbox of our output node
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
    fn emit(&mut self, args: EmitOpArgs<'c, ExitBlock, H>) -> Result<()> {
        let outputs = self.take_outputs()?;
        let (bb, inputs_rmb) = self.bbs.get(&args.node().generalise()).unwrap();
        self.context.build_positioned(*bb, |context| {
            let builder = context.builder();
            outputs.finish(builder, inputs_rmb.read_vec(builder, [])?)
        })
    }
}
