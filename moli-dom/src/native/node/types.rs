use smol_str::{SmolStr, SmolStrBuilder};
use std::sync::Arc;

#[derive(Debug, Clone)]
pub struct Text {
    data: TextData,
}

#[derive(Debug, Clone)]
enum TextData {
    Compact(SmolStr),
    // Retain capacity while the parser appends character tokens. This fits
    // within the existing NodeData storage alongside the compact form.
    Appending(String),
}

impl Text {
    pub fn new(data: impl Into<SmolStr>) -> Self {
        Self {
            data: TextData::Compact(data.into()),
        }
    }

    pub fn data(&self) -> &str {
        match &self.data {
            TextData::Compact(data) => data.as_str(),
            TextData::Appending(data) => data.as_str(),
        }
    }

    pub(crate) fn append_data(&mut self, data: &str) {
        if data.is_empty() {
            return;
        }
        match &mut self.data {
            TextData::Appending(buffer) => buffer.push_str(data),
            // SmolStr guarantees inline storage for up to 23 UTF-8 bytes.
            // Keep split character tokens inline without allocating a String.
            TextData::Compact(current) if current.len() + data.len() <= 23 => {
                let mut buffer = SmolStrBuilder::new();
                buffer.push_str(current.as_str());
                buffer.push_str(data);
                *current = buffer.finish();
            }
            TextData::Compact(current) => {
                let mut buffer = String::with_capacity(current.len() + data.len());
                buffer.push_str(current.as_str());
                buffer.push_str(data);
                self.data = TextData::Appending(buffer);
            }
        }
    }

    pub(crate) fn finish_appending(&mut self) {
        if let TextData::Appending(buffer) = &mut self.data {
            self.data = TextData::Compact(std::mem::take(buffer).into());
        }
    }

    pub(crate) fn shared_data(&self) -> Arc<str> {
        match &self.data {
            TextData::Compact(data) => Arc::from(data.clone()),
            TextData::Appending(data) => Arc::from(data.as_str()),
        }
    }

    pub fn set_data(&mut self, data: impl Into<Arc<str>>) {
        self.data = TextData::Compact(SmolStr::from(data.into()));
    }
}

#[cfg(test)]
mod tests {
    use super::{Text, TextData};
    use std::sync::Arc;

    #[test]
    fn text_inlines_short_data() {
        let text = Text::new("short text");

        assert!(matches!(text.data, TextData::Compact(data) if !data.is_heap_allocated()));
    }

    #[test]
    fn appended_short_text_stays_inline() {
        let mut text = Text::new("a");
        text.append_data("<");
        text.append_data("b");

        assert_eq!(text.data(), "a<b");
        assert!(matches!(text.data, TextData::Compact(data) if !data.is_heap_allocated()));
    }

    #[test]
    fn appended_text_keeps_utf8_inline_until_it_exceeds_capacity() {
        let mut text = Text::new("é".repeat(10));
        text.append_data("界");

        assert_eq!(text.data(), format!("{}界", "é".repeat(10)));
        assert!(matches!(&text.data, TextData::Compact(data) if !data.is_heap_allocated()));

        text.append_data("!");
        assert_eq!(text.data(), format!("{}界!", "é".repeat(10)));
        text.finish_appending();
        assert!(Arc::ptr_eq(&text.shared_data(), &text.shared_data()));
    }

    #[test]
    fn text_shares_long_data_when_requested() {
        let text = Text::new("x".repeat(64));

        let first = text.shared_data();
        let second = text.shared_data();
        assert!(Arc::ptr_eq(&first, &second));
    }

    #[test]
    fn appended_text_shares_completed_backing_without_changing_existing_snapshots() {
        let source = "x".repeat(64);
        let mut text = Text::new(source.clone());
        let original = text.shared_data();

        text.append_data("suffix");
        text.finish_appending();

        let first = text.shared_data();
        let second = text.shared_data();
        assert_eq!(original.as_ref(), source);
        assert_eq!(first.as_ref(), format!("{source}suffix"));
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
