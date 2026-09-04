use smol_str::SmolStr;
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Text {
    data: SmolStr,
}

impl Text {
    pub fn new(data: impl Into<SmolStr>) -> Self {
        Self { data: data.into() }
    }

    pub fn data(&self) -> &str {
        self.data.as_str()
    }

    pub(crate) fn shared_data(&self) -> Arc<str> {
        Arc::from(self.data.clone())
    }

    pub fn set_data(&mut self, data: impl Into<Arc<str>>) {
        self.data = SmolStr::from(data.into());
    }
}

#[cfg(test)]
mod tests {
    use super::Text;
    use std::sync::Arc;

    #[test]
    fn text_inlines_short_data() {
        let text = Text::new("short text");

        assert!(!text.data.is_heap_allocated());
    }

    #[test]
    fn text_shares_long_data_when_requested() {
        let text = Text::new("x".repeat(64));

        let first = text.shared_data();
        let second = text.shared_data();
        assert!(Arc::ptr_eq(&first, &second));
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
