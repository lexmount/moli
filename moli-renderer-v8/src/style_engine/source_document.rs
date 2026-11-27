use crate::document_runtime::DomHandle;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct StyleSourceDocumentContext<'a> {
    root_document: DomHandle,
    child_documents: &'a [DomHandle],
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OwnedStyleSourceDocumentContext {
    root_document: DomHandle,
    child_documents: Vec<DomHandle>,
}

impl OwnedStyleSourceDocumentContext {
    pub(crate) fn new(root_document: DomHandle) -> Self {
        Self {
            root_document,
            child_documents: Vec::new(),
        }
    }

    pub(crate) fn with_child_documents(
        mut self,
        child_documents: impl IntoIterator<Item = DomHandle>,
    ) -> Self {
        self.child_documents.extend(child_documents);
        self
    }

    pub(crate) fn as_ref(&self) -> StyleSourceDocumentContext<'_> {
        StyleSourceDocumentContext {
            root_document: self.root_document,
            child_documents: &self.child_documents,
        }
    }
}

impl<'a> StyleSourceDocumentContext<'a> {
    #[cfg(test)]
    pub(crate) fn for_root_document(root_document: DomHandle) -> Self {
        Self {
            root_document,
            child_documents: &[],
        }
    }

    pub(crate) fn documents(self) -> Vec<DomHandle> {
        let mut documents = Vec::with_capacity(1 + self.child_documents.len());
        push_unique_document(&mut documents, self.root_document);
        for &document in self.child_documents {
            push_unique_document(&mut documents, document);
        }
        documents
    }

    pub(crate) fn documents_with_owner(self, owner_document: DomHandle) -> Vec<DomHandle> {
        let mut documents = Vec::with_capacity(2);
        push_unique_document(&mut documents, self.root_document);
        push_unique_document(&mut documents, owner_document);
        documents
    }
}

fn push_unique_document(documents: &mut Vec<DomHandle>, document: DomHandle) {
    if !documents.contains(&document) {
        documents.push(document);
    }
}
