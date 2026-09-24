mod main_dynamic_import_graph;
mod main_modulepreload;
mod main_parser_module_graph;
mod main_runtime_module_graph;
mod owner;
mod sender;
mod terminal;
mod turn;

use std::{error::Error, fmt};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct RendererResourceCompletionRouteClosed;

impl fmt::Display for RendererResourceCompletionRouteClosed {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("renderer resource-completion route is closed")
    }
}

impl Error for RendererResourceCompletionRouteClosed {}

pub(crate) use self::main_dynamic_import_graph::{
    MainDynamicImportGraphFetchCompletion, MainDynamicImportGraphFetchTarget,
};
pub(crate) use self::main_modulepreload::{
    MainModulepreloadFetchCompletion, MainModulepreloadFetchTarget,
};
pub(crate) use self::main_parser_module_graph::{
    MainParserModuleGraphFetchCompletion, MainParserModuleGraphFetchTarget,
};
pub(crate) use self::main_runtime_module_graph::{
    MainRuntimeModuleGraphFetchCompletion, MainRuntimeModuleGraphFetchTarget,
};
pub(crate) use self::owner::{
    RendererPageResourceCompletionLocalOwner, RendererPageResourceCompletionOwner,
};
pub(crate) use self::sender::RendererPageResourceCompletionSender;
#[cfg(test)]
pub(crate) use self::sender::RendererPageResourceCompletionTestSource;
pub(crate) use self::terminal::{RendererPageResourceCompletion, RendererPageResourceTerminal};
pub(crate) use self::turn::{
    PageResourceCompletionBodyActivity, PageResourceCompletionDocumentEffect,
    PageResourceCompletionOutputEffect, PageResourceCompletionPostCheckpointEffect,
    PageResourceCompletionTurnAction, PageResourceCompletionTurnOutcome,
};
