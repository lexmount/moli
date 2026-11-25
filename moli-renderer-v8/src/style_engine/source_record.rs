use super::{source::store::StyloStylesheetSource, source_id::StyleSourceId};

pub(super) struct RetainedStylesheetSourceRecord<'a> {
    id: StyleSourceId,
    source: RetainedStylesheetSourceRecordSource<'a>,
}

enum RetainedStylesheetSourceRecordSource<'a> {
    OwnerStyleSheet(&'a StyloStylesheetSource),
    Registered(&'a StyloStylesheetSource),
}

impl<'a> RetainedStylesheetSourceRecord<'a> {
    pub(super) fn owner_style_sheet(id: StyleSourceId, source: &'a StyloStylesheetSource) -> Self {
        Self {
            id,
            source: RetainedStylesheetSourceRecordSource::OwnerStyleSheet(source),
        }
    }

    pub(super) fn registered(id: StyleSourceId, source: &'a StyloStylesheetSource) -> Self {
        Self {
            id,
            source: RetainedStylesheetSourceRecordSource::Registered(source),
        }
    }

    pub(super) fn id(&self) -> &StyleSourceId {
        &self.id
    }

    pub(super) fn to_stylo_source(&self) -> StyloStylesheetSource {
        let source = match &self.source {
            RetainedStylesheetSourceRecordSource::OwnerStyleSheet(source) => (*source).clone(),
            RetainedStylesheetSourceRecordSource::Registered(source) => (*source).clone(),
        };
        source.with_source_id(Some(self.id.clone()))
    }
}
