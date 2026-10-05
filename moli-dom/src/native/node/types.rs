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
    target: Box<str>,
    data: Box<str>,
}

impl ProcessingInstruction {
    pub fn new(target: String, data: String) -> Self {
        Self {
            target: target.into_boxed_str(),
            data: data.into_boxed_str(),
        }
    }

    pub fn target(&self) -> &str {
        &self.target
    }

    pub fn data(&self) -> &str {
        &self.data
    }

    pub fn set_data(&mut self, data: impl Into<String>) {
        self.data = data.into().into_boxed_str();
    }
}
