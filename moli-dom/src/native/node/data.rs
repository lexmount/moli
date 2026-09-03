use super::super::document::{Document, DocumentFragment, DocumentType};
use super::super::element::Element;
use super::{CDataSection, Comment, NodeType, ProcessingInstruction, Text};

#[derive(Debug, Clone)]
pub enum NodeData {
    // A document is unique and large. Keeping it out of line prevents its
    // base-URL and metadata payload from defining the size of every node kind.
    Document(Box<Document>),
    DocumentType(DocumentType),
    Element(Element),
    Text(Text),
    CDataSection(CDataSection),
    Comment(Comment),
    ProcessingInstruction(ProcessingInstruction),
    DocumentFragment(DocumentFragment),
}

impl NodeData {
    pub fn node_type(&self) -> NodeType {
        match self {
            Self::Document(_) => NodeType::Document,
            Self::DocumentType(_) => NodeType::DocumentType,
            Self::Element(_) => NodeType::Element,
            Self::Text(_) => NodeType::Text,
            Self::CDataSection(_) => NodeType::CDataSection,
            Self::Comment(_) => NodeType::Comment,
            Self::ProcessingInstruction(_) => NodeType::ProcessingInstruction,
            Self::DocumentFragment(_) => NodeType::DocumentFragment,
        }
    }

    pub fn node_name(&self) -> String {
        match self {
            Self::Document(_) => "#document".to_owned(),
            Self::DocumentType(document_type) => document_type.name().to_owned(),
            Self::Element(element) => element.node_name(),
            Self::Text(_) => "#text".to_owned(),
            Self::CDataSection(_) => "#cdata-section".to_owned(),
            Self::Comment(_) => "#comment".to_owned(),
            Self::ProcessingInstruction(processing_instruction) => {
                processing_instruction.target().to_owned()
            }
            Self::DocumentFragment(_) => "#document-fragment".to_owned(),
        }
    }

    pub fn is_document_fragment(&self) -> bool {
        matches!(self, Self::DocumentFragment(_))
    }

    pub fn as_document(&self) -> Option<&Document> {
        match self {
            Self::Document(document) => Some(document.as_ref()),
            _ => None,
        }
    }

    pub fn as_document_mut(&mut self) -> Option<&mut Document> {
        match self {
            Self::Document(document) => Some(document.as_mut()),
            _ => None,
        }
    }

    pub fn as_document_type(&self) -> Option<&DocumentType> {
        match self {
            Self::DocumentType(document_type) => Some(document_type),
            _ => None,
        }
    }

    pub fn as_element(&self) -> Option<&Element> {
        match self {
            Self::Element(element) => Some(element),
            _ => None,
        }
    }

    pub fn as_element_mut(&mut self) -> Option<&mut Element> {
        match self {
            Self::Element(element) => Some(element),
            _ => None,
        }
    }

    pub fn as_text(&self) -> Option<&Text> {
        match self {
            Self::Text(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_text_mut(&mut self) -> Option<&mut Text> {
        match self {
            Self::Text(text) => Some(text),
            _ => None,
        }
    }

    pub fn as_cdata_section(&self) -> Option<&CDataSection> {
        match self {
            Self::CDataSection(cdata) => Some(cdata),
            _ => None,
        }
    }

    pub fn as_cdata_section_mut(&mut self) -> Option<&mut CDataSection> {
        match self {
            Self::CDataSection(cdata) => Some(cdata),
            _ => None,
        }
    }

    pub fn as_comment(&self) -> Option<&Comment> {
        match self {
            Self::Comment(comment) => Some(comment),
            _ => None,
        }
    }

    pub fn as_comment_mut(&mut self) -> Option<&mut Comment> {
        match self {
            Self::Comment(comment) => Some(comment),
            _ => None,
        }
    }

    pub fn as_processing_instruction(&self) -> Option<&ProcessingInstruction> {
        match self {
            Self::ProcessingInstruction(processing_instruction) => Some(processing_instruction),
            _ => None,
        }
    }

    pub fn as_processing_instruction_mut(&mut self) -> Option<&mut ProcessingInstruction> {
        match self {
            Self::ProcessingInstruction(processing_instruction) => Some(processing_instruction),
            _ => None,
        }
    }

    pub fn as_document_fragment(&self) -> Option<&DocumentFragment> {
        match self {
            Self::DocumentFragment(fragment) => Some(fragment),
            _ => None,
        }
    }

    pub fn as_document_fragment_mut(&mut self) -> Option<&mut DocumentFragment> {
        match self {
            Self::DocumentFragment(fragment) => Some(fragment),
            _ => None,
        }
    }
}
