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
    target: Box<str>,
    data: DomStringValue,
}

impl ProcessingInstruction {
    pub fn new(target: String, data: impl Into<DomStringValue>) -> Self {
        Self {
            target: target.into_boxed_str(),
            data: data.into(),
        }
    }

    pub fn target(&self) -> &str {
        &self.target
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
