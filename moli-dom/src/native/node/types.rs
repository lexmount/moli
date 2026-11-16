use super::super::DomStringValue;

#[derive(Debug, Clone)]
pub struct Text {
    data: DomStringValue,
}

impl Text {
    pub fn new(data: impl Into<DomStringValue>) -> Self {
        Self { data: data.into() }
    }

    pub fn data(&self) -> &str {
        self.data.as_str_lossy()
    }

    pub fn value(&self) -> &DomStringValue {
        &self.data
    }

    pub(crate) fn value_mut(&mut self) -> &mut DomStringValue {
        &mut self.data
    }

    pub fn set_data(&mut self, data: impl Into<DomStringValue>) {
        self.data = data.into();
    }

    pub(crate) fn append_data(&mut self, data: &str) {
        self.data.append_str(data);
    }
}

#[derive(Debug, Clone)]
pub struct CDataSection {
    data: DomStringValue,
}

impl CDataSection {
    pub fn new(data: impl Into<DomStringValue>) -> Self {
        Self { data: data.into() }
    }

    pub fn data(&self) -> &str {
        self.data.as_str_lossy()
    }

    pub fn value(&self) -> &DomStringValue {
        &self.data
    }

    pub(crate) fn value_mut(&mut self) -> &mut DomStringValue {
        &mut self.data
    }

    pub fn set_data(&mut self, data: impl Into<DomStringValue>) {
        self.data = data.into();
    }
}

#[derive(Debug, Clone)]
pub struct Comment {
    data: DomStringValue,
}

impl Comment {
    pub fn new(data: impl Into<DomStringValue>) -> Self {
        Self { data: data.into() }
    }

    pub fn data(&self) -> &str {
        self.data.as_str_lossy()
    }

    pub fn value(&self) -> &DomStringValue {
        &self.data
    }

    pub(crate) fn value_mut(&mut self) -> &mut DomStringValue {
        &mut self.data
    }

    pub fn set_data(&mut self, data: impl Into<DomStringValue>) {
        self.data = data.into();
    }
}

#[derive(Debug, Clone)]
pub struct ProcessingInstruction {
    data: DomStringValue,
    metadata: Box<ProcessingInstructionMetadata>,
}

// Keep PI-only metadata out of the common NodeData arena record.
#[derive(Debug, Clone)]
struct ProcessingInstructionMetadata {
    target: Box<str>,
    attributes: super::ProcessingInstructionAttributes,
}

impl ProcessingInstruction {
    pub fn new(target: String, data: impl Into<DomStringValue>) -> Self {
        let data = data.into();
        let attributes = super::ProcessingInstructionAttributes::from_data(&data);
        Self {
            data,
            metadata: Box::new(ProcessingInstructionMetadata {
                target: target.into_boxed_str(),
                attributes,
            }),
        }
    }

    pub fn target(&self) -> &str {
        &self.metadata.target
    }

    pub fn data(&self) -> &str {
        self.data.as_str_lossy()
    }

    pub fn value(&self) -> &DomStringValue {
        &self.data
    }

    pub(crate) fn value_mut(&mut self) -> &mut DomStringValue {
        &mut self.data
    }

    pub fn set_data(&mut self, data: impl Into<DomStringValue>) {
        self.data = data.into();
        self.refresh_attributes_from_data();
    }

    pub fn attributes(&self) -> &super::ProcessingInstructionAttributes {
        &self.metadata.attributes
    }

    pub(crate) fn replace_attributes(
        &mut self,
        attributes: super::ProcessingInstructionAttributes,
    ) {
        self.metadata.attributes = attributes;
    }

    pub(crate) fn refresh_attributes_from_data(&mut self) {
        self.metadata.attributes = super::ProcessingInstructionAttributes::from_data(&self.data);
    }
}
