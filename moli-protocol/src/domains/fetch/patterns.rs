use crate::conn::{FetchInterceptionPattern, FetchRequestStage, FetchResourceTypeFilter};

use super::params::RequestPattern;

pub(super) fn supported_pattern_config(
    patterns: &[RequestPattern],
) -> Result<Vec<FetchInterceptionPattern>, ()> {
    if patterns.is_empty() {
        return Ok(vec![FetchInterceptionPattern {
            url_pattern: "*".to_owned(),
            resource_type_filter: None,
            request_stage: FetchRequestStage::Request,
        }]);
    }
    patterns
        .iter()
        .map(|pattern| {
            let request_stage = FetchRequestStage::parse(&pattern.request_stage).ok_or(())?;
            let resource_type_filter = match pattern.resource_type.as_deref() {
                None => None,
                Some(resource_type) => {
                    let filter = FetchResourceTypeFilter::parse(resource_type).ok_or(())?;
                    if filter.interception_classes().is_empty() {
                        return Err(());
                    }
                    Some(filter)
                }
            };
            Ok(FetchInterceptionPattern {
                url_pattern: pattern.url_pattern.clone(),
                resource_type_filter,
                request_stage,
            })
        })
        .collect::<Result<Vec<_>, _>>()
}

pub(super) fn validate_request_id(request_id: &str) -> Result<(), ()> {
    if request_id
        .strip_prefix("INT-SUB-")
        .is_some_and(|suffix| !suffix.is_empty() && suffix.chars().all(|ch| ch.is_ascii_digit()))
        || request_id.strip_prefix("INT-").is_some_and(|suffix| {
            !suffix.is_empty() && suffix.chars().all(|ch| ch.is_ascii_digit())
        })
    {
        Ok(())
    } else {
        Err(())
    }
}
