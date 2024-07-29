use std::path::{Path, PathBuf};

use hugr::{
    ops::{FuncDefn, OpType},
    HugrView, Node,
};
use inkwell::{
    context::Context,
    debug_info::{DICompileUnit, DILocation, DIScope, DISubprogram, DebugInfoBuilder},
    module::Module,
};
use serde::{Deserialize, Serialize};

use crate::{emit::Namer, fat::FatNode};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModuleInfo {
    file: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LocationInfo {
    file: String,
    line: u32,
    col: u32,
}

pub trait DebugHugrView: HugrView {
    fn get_module_info(&self, node: Node) -> Option<ModuleInfo> {
        self.get_metadata(node, "di")
            .and_then(|di| serde_json::from_value(di.clone()).ok())
    }

    fn get_location_info(&self, node: Node) -> Option<LocationInfo> {
        self.get_metadata(node, "di")
            .and_then(|di| serde_json::from_value(di.clone()).ok())
    }
}

impl<T: HugrView> DebugHugrView for T {}

fn split_path(path: impl AsRef<Path>) -> Option<(String, String)> {
    let dir = path.as_ref().parent()?.to_str()?.to_owned();
    let file = path.as_ref().file_name()?.to_str()?.to_owned();
    Some((dir, file))
}

pub fn module_debug_info<'c, H: DebugHugrView>(
    node: FatNode<'c, hugr::ops::Module, H>,
    module: &Module<'c>,
) -> (DebugInfoBuilder<'c>, DICompileUnit<'c>) {
    let (dir, file) = node
        .hugr()
        .get_module_info(node.node())
        .and_then(|mi| {
            dbg!(&mi);
            split_path(PathBuf::from(mi.file))
        })
        .unwrap_or(("".to_owned(), "".to_owned()));
    module.create_debug_info_builder(
        true,                                        // allow_unresolved
        inkwell::debug_info::DWARFSourceLanguage::C, // language
        &file,                                       // filename
        &dir,                                        // directory
        "guppy",                                     // produer
        false,                                       // is_optimised
        "",                                          //flags
        0,                                           // runtime_ver
        "",                                          //split_name
        inkwell::debug_info::DWARFEmissionKind::Full,
        0,     // dwo_id
        false, // split_debug_inlining
        false, //debug_info_for_profiling
        "",    // sysroot
        "",
    ) // dk
}

pub fn func_debug_info<'c, H: DebugHugrView>(
    builder: &DebugInfoBuilder<'c>,
    namer: &Namer,
    scope: DIScope<'c>,
    node: FatNode<'c, FuncDefn, H>,
) -> DISubprogram<'c> {
    let (dir, filename, line, _) = node
        .hugr()
        .get_location_info(node.node())
        .and_then(|li| {
            let (dir, file) = split_path(PathBuf::from(li.file))?;
            Some((dir, file, li.line, li.col))
        })
        .unwrap_or(("".to_owned(), "".to_owned(), 0, 0));
    let di_file = builder.create_file(&filename, &dir);
    let di_subroutine_type = builder.create_subroutine_type(di_file, None, &[], 0);
    let linkage_name = namer.name_func(&node.name, node.node());
    builder.create_function(
        scope,
        &node.name,
        Some(&linkage_name),
        di_file,
        line,
        di_subroutine_type,
        false, // is_local_to_unit
        true,  // is_definition
        line,
        inkwell::debug_info::DIFlags::default(),
        false, // is_optimised
    )
}

pub fn op_debug_location<'c, H: DebugHugrView>(
    context: &'c Context,
    builder: &DebugInfoBuilder<'c>,
    scope: DIScope<'c>,
    node: FatNode<'c, OpType, H>,
) -> DILocation<'c> {
    let (line, col) = node
        .hugr()
        .get_location_info(node.node())
        .map(|li| (li.line, li.col))
        .unwrap_or((0, 0));
    builder.create_debug_location(context, line, col, scope, None)
}
