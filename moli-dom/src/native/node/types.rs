use super::super::DomStringValue;
use super::ProcessingInstructionAttributes;

#[derive(Debug, Clone)]
pub struct Text {
    // Text shares the larger NodeData enum storage with elements, so retaining
    // String capacity does not enlarge a Node. Parser character tokens can then
    // append without reallocating and copying the full text for every token.
    data: String,
}

impl Text {
    pub fn new(data: String) -> Self {
        Self { data }
    }

    pub fn data(&self) -> &str {
        &self.data
    }

    pub fn set_data(&mut self, data: impl Into<String>) {
        self.data = data.into();
    }

    pub(crate) fn append_data(&mut self, data: &str) {
        self.data.push_str(data);
    }
}

#[derive(Debug, Clone)]
pub struct CDataSection {
    data: Box<str>,
}

impl CDataSection {
    pub fn new(data: String) -> Self {
        Self {
            data: data.into_boxed_str(),
        }
    }

    pub fn data(&self) -> &str {
        &self.data
    }

    pub fn set_data(&mut self, data: impl Into<String>) {
        self.data = data.into().into_boxed_str();
    }
}

#[derive(Debug, Clone)]
pub struct Comment {
    data: Box<str>,
}

impl Comment {
    pub fn new(data: String) -> Self {
        Self {
            data: data.into_boxed_str(),
        }
    }

    pub fn data(&self) -> &str {
        &self.data
    }

    pub fn set_data(&mut self, data: impl Into<String>) {
        self.data = data.into().into_boxed_str();
    }
}

#[derive(Debug, Clone)]
pub struct ProcessingInstruction {
    data: DomStringValue,
    metadata: Box<ProcessingInstructionMetadata>,
}

// Keep the target and PI-only attribute map out of the common NodeData record.
#[derive(Debug, Clone)]
struct ProcessingInstructionMetadata {
    target: Box<str>,
    attributes: ProcessingInstructionAttributes,
}

impl ProcessingInstruction {
    pub fn new(target: String, data: String) -> Self {
        let data = DomStringValue::from(data);
        Self {
            metadata: Box::new(ProcessingInstructionMetadata {
                target: target.into_boxed_str(),
                attributes: ProcessingInstructionAttributes::from_data(&data),
            }),
            data,
        }
    }

    pub fn target(&self) -> &str {
        &self.metadata.target
    }

    pub fn data(&self) -> &str {
        self.data.as_str_lossy()
    }

    pub fn data_value(&self) -> &DomStringValue {
        &self.data
    }

    pub fn set_data(&mut self, data: impl Into<DomStringValue>) {
        self.data = data.into();
        self.metadata.attributes = ProcessingInstructionAttributes::from_data(&self.data);
    }

    pub fn attributes(&self) -> &ProcessingInstructionAttributes {
        &self.metadata.attributes
    }

    pub(crate) fn replace_attributes(
        &mut self,
        attributes: ProcessingInstructionAttributes,
    ) -> bool {
        let serialized = attributes.serialize();
        let changed = self.data != serialized;
        self.data = serialized;
        self.metadata.attributes = attributes;
        changed
    }
}
