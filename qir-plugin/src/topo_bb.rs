use itertools::Itertools as _;
use llvm_plugin::inkwell::{basic_block::BasicBlock, values::FunctionValue};

type G<'c> = petgraph::graph::Graph<BasicBlock<'c>, ()>;

pub fn topo_bbs<'c>(func: FunctionValue<'c>) -> Option<Vec<BasicBlock<'c>>> {
    let mut g = G::new();
    let bbs = func.get_basic_blocks();
    let mut bbs_map = std::collections::HashMap::new();
    for bb in bbs.iter() {
        let node = g.add_node(*bb);
        bbs_map.insert(bb, node);
    }
    for bb in bbs.iter() {
        let node = bbs_map[&bb];
        for succ in bb.get_terminator().unwrap().get_operands().filter_map(|x| x.and_then(|x| x.right())) {
            let succ_node = bbs_map[&succ];
            g.add_edge(node, succ_node, ());
        }
    }
    let order = petgraph::algo::toposort(&g, None).ok()?;
    Some(order.into_iter().map(move |node| g[node]).collect_vec())
}
