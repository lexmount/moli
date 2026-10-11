//! Sensor declarations remain local to a Document. Delegation snapshots the
//! parent's effective permission into the child's inherited policy; it must not
//! reinterpret an ancestor's header as the child's own declaration.

use super::{OriginAllowlist, iframe_allow_feature, response_origin_allowlist};
use url::{Origin, Url};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SensorPermission {
    Accelerometer,
    Gyroscope,
    Magnetometer,
}

impl SensorPermission {
    pub(crate) const ALL: [Self; 3] = [Self::Accelerometer, Self::Gyroscope, Self::Magnetometer];

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Accelerometer => "accelerometer",
            Self::Gyroscope => "gyroscope",
            Self::Magnetometer => "magnetometer",
        }
    }

    pub(super) fn from_name(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|sensor| sensor.name().eq_ignore_ascii_case(name))
    }

    const fn index(self) -> usize {
        match self {
            Self::Accelerometer => 0,
            Self::Gyroscope => 1,
            Self::Magnetometer => 2,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct SensorPolicy {
    inherited: bool,
    declarations: Vec<(Origin, OriginAllowlist)>,
}

impl Default for SensorPolicy {
    fn default() -> Self {
        Self {
            inherited: true,
            declarations: Vec::new(),
        }
    }
}

impl SensorPolicy {
    fn allows(&self, origin: &Origin) -> bool {
        self.inherited
            && self.declarations.iter().all(|(anchor, allowed)| {
                allowed.matches_self && origin == anchor || allowed.origins.contains(origin)
            })
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct SensorPermissionsPolicy {
    origin: Option<Origin>,
    features: [SensorPolicy; 3],
}

impl Default for SensorPermissionsPolicy {
    fn default() -> Self {
        Self {
            origin: None,
            features: std::array::from_fn(|_| SensorPolicy::default()),
        }
    }
}

impl SensorPermissionsPolicy {
    pub(super) fn enabled(&self, sensor: SensorPermission) -> bool {
        let feature = &self.features[sensor.index()];
        self.origin.as_ref().map_or_else(
            || feature.inherited && feature.declarations.is_empty(),
            |origin| feature.allows(origin),
        )
    }

    pub(super) fn declare(&mut self, sensor: SensorPermission, allowlist: &str, url: &Url) {
        self.features[sensor.index()].declarations = response_origin_allowlist(allowlist)
            .map(|list| (url.origin(), list))
            .into_iter()
            .collect();
        self.bind_document_origin(&url.origin());
    }

    pub(super) fn intersect(&self, other: &Self) -> Self {
        Self {
            origin: other.origin.clone().or_else(|| self.origin.clone()),
            features: std::array::from_fn(|index| SensorPolicy {
                inherited: self.features[index].inherited && other.features[index].inherited,
                declarations: self.features[index]
                    .declarations
                    .iter()
                    .chain(&other.features[index].declarations)
                    .cloned()
                    .collect(),
            }),
        }
    }

    pub(super) fn for_document_origin(&self, origin: &Origin) -> Self {
        let mut policy = self.clone();
        policy.bind_document_origin(origin);
        policy
    }

    fn bind_document_origin(&mut self, origin: &Origin) {
        // An inherited policy contains feature values, not declarations. It
        // needs no origin anchor; retaining an unused opaque URL origin would
        // make otherwise identical policies compare unequal during initial
        // child Window reuse.
        self.origin = self
            .features
            .iter()
            .any(|feature| !feature.declarations.is_empty())
            .then(|| origin.clone());
    }

    pub(super) fn delegated_to_child(
        &self,
        parent: &Origin,
        child: &Origin,
        source: &Origin,
        allow: Option<&str>,
    ) -> Self {
        Self {
            origin: None,
            features: std::array::from_fn(|index| {
                let feature = &self.features[index];
                let container = iframe_allow_feature(
                    allow,
                    SensorPermission::ALL[index].name(),
                    parent,
                    child,
                    source,
                )
                .unwrap_or(parent == child);
                SensorPolicy {
                    inherited: feature.allows(parent) && feature.allows(child) && container,
                    declarations: Vec::new(),
                }
            }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::permissions_policy::DocumentPermissionsPolicy;

    fn origin(host: &str) -> Origin {
        Url::parse(&format!("https://{host}/")).unwrap().origin()
    }
    fn header(value: &str) -> DocumentPermissionsPolicy {
        DocumentPermissionsPolicy::from_navigation_response_headers(
            &[("Permissions-Policy".into(), value.as_bytes().to_vec())],
            &Url::parse("https://parent.test/").unwrap(),
        )
    }

    #[test]
    fn inherited_sensor_policy_identity_does_not_retain_unused_opaque_origins() {
        let policy = SensorPermissionsPolicy::default();
        let first = Url::parse("data:text/html,creator").unwrap().origin();
        let second = Url::parse("data:text/html,creator").unwrap().origin();
        assert_ne!(first, second);
        assert_eq!(policy.for_document_origin(&first), policy);
        assert_eq!(policy.for_document_origin(&second), policy);
        assert_eq!(
            policy.delegated_to_child(&first, &first, &first, None),
            policy
        );
    }
    fn child(
        policy: &DocumentPermissionsPolicy,
        parent: &str,
        destination: &str,
        source: &str,
        allow: Option<&str>,
    ) -> DocumentPermissionsPolicy {
        policy.delegated_to_child(
            &origin(parent),
            &origin(destination),
            &origin(source),
            allow,
            false,
        )
    }

    #[test]
    fn sensor_policy_defaults_to_self_and_requires_explicit_cross_origin_delegation() {
        let root = DocumentPermissionsPolicy::default();
        let same = child(&root, "parent.test", "parent.test", "parent.test", None);
        let cross = child(&root, "parent.test", "child.test", "child.test", None);
        let delegated = child(
            &root,
            "parent.test",
            "child.test",
            "child.test",
            Some("accelerometer *; gyroscope *; magnetometer *"),
        );
        for sensor in SensorPermission::ALL {
            assert!(root.sensor_enabled(sensor));
            assert!(same.sensor_enabled(sensor));
            assert!(!cross.sensor_enabled(sensor));
            assert!(delegated.sensor_enabled(sensor));
        }
    }

    #[test]
    fn sensor_headers_and_container_restrictions_apply_independently() {
        let root = header("accelerometer=(), gyroscope=(self), magnetometer=*");
        assert!(!root.sensor_enabled(SensorPermission::Accelerometer));
        assert!(root.sensor_enabled(SensorPermission::Gyroscope));
        let frame = child(
            &root,
            "parent.test",
            "child.test",
            "child.test",
            Some("accelerometer *; gyroscope *; magnetometer *"),
        );
        assert!(!frame.sensor_enabled(SensorPermission::Accelerometer));
        assert!(!frame.sensor_enabled(SensorPermission::Gyroscope));
        assert!(frame.sensor_enabled(SensorPermission::Magnetometer));
    }

    #[test]
    fn sensor_container_src_remains_bound_to_declared_source_after_redirect() {
        let root = DocumentPermissionsPolicy::default();
        let allowed = child(
            &root,
            "parent.test",
            "source.test",
            "source.test",
            Some("accelerometer"),
        );
        let redirected = child(
            &root,
            "parent.test",
            "redirect.test",
            "source.test",
            Some("accelerometer"),
        );
        assert!(allowed.sensor_enabled(SensorPermission::Accelerometer));
        assert!(!redirected.sensor_enabled(SensorPermission::Accelerometer));
    }

    #[test]
    fn sensor_response_cannot_reenable_a_denied_inherited_feature() {
        let denied = child(
            &DocumentPermissionsPolicy::default(),
            "parent.test",
            "child.test",
            "child.test",
            None,
        );
        let response = header("accelerometer=*, gyroscope=*, magnetometer=*")
            .for_document_origin(&origin("child.test"));
        let combined = denied.intersect(&response);
        for sensor in SensorPermission::ALL {
            assert!(!combined.sensor_enabled(sensor));
        }
    }

    #[test]
    fn sensor_parent_must_allow_itself_before_delegating() {
        let root = header("accelerometer=(\"https://child.test\")");
        let frame = child(
            &root,
            "parent.test",
            "child.test",
            "child.test",
            Some("accelerometer *"),
        );
        assert!(!root.sensor_enabled(SensorPermission::Accelerometer));
        assert!(!frame.sensor_enabled(SensorPermission::Accelerometer));
    }

    #[test]
    fn sensor_delegation_inherits_a_value_without_copying_ancestor_declarations() {
        let root = header("accelerometer=(self \"https://child.test\")");
        let frame = child(
            &root,
            "parent.test",
            "child.test",
            "child.test",
            Some("accelerometer *"),
        );
        assert!(frame.sensor_enabled(SensorPermission::Accelerometer));
        let grandchild = child(
            &frame,
            "child.test",
            "grandchild.test",
            "grandchild.test",
            Some("accelerometer *"),
        );
        assert!(grandchild.sensor_enabled(SensorPermission::Accelerometer));
        let response = header("accelerometer=()").for_document_origin(&origin("child.test"));
        let restricted = frame.intersect(&response);
        assert!(
            !child(
                &restricted,
                "child.test",
                "grandchild.test",
                "grandchild.test",
                Some("accelerometer *")
            )
            .sensor_enabled(SensorPermission::Accelerometer)
        );
    }
}
