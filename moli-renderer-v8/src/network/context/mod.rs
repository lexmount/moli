mod document;
mod registry;
mod request_context;
mod worker;

#[cfg(test)]
mod tests;

pub(crate) use document::{
    DocumentResourceAuthoritySource, DocumentResourceLoaderAuthority,
    DocumentResourceLoaderBootstrap, DocumentResourceLoaderIdentity,
};
pub use document::{
    DocumentResourceLoader, DocumentResourceLoaderDiagnostics, DocumentResourceLoaderState,
};
pub(crate) use registry::DocumentResourceLoaderRegistry;
pub(crate) use request_context::{DocumentFetchContext, SubresourceRequestEnvironment};
#[cfg(test)]
pub(crate) use worker::WorkerResourceLoaderState;
pub(crate) use worker::{WorkerResourceLoader, WorkerResourceOwner};
