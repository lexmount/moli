use super::*;

fn policy_container_with_permissions_policy(
    url: &str,
    header: &str,
) -> crate::document_runtime::DocumentPolicyContainer {
    crate::document_runtime::DocumentPolicyContainer::from_navigation_response_headers(
        &[("Permissions-Policy".to_owned(), header.as_bytes().to_vec())],
        &url::Url::parse(url).expect("policy test URL"),
    )
}

#[test]
fn permissions_query_reports_denied_for_features_disabled_by_response_policy() {
    let url = "https://permissions-policy-query.test/";
    let mut vm = new_storage_test_vm(url);
    vm.set_main_navigation_policy_container(policy_container_with_permissions_policy(
        url,
        "camera=(), microphone=(self), geolocation=*",
    ));

    vm.eval(
        r#"
      (() => {
        globalThis.__policyQuery = "pending";
        Promise.all([
          navigator.permissions.query({ name: "camera" }),
          navigator.permissions.query({ name: "microphone" }),
          navigator.permissions.query({ name: "geolocation" }),
          navigator.permissions.query({ name: "notifications" })
        ]).then(statuses => {
          globalThis.__policyQuery = statuses.map(status => status.state).join(",");
        });
      })()
    "#,
    )
    .expect("permissions policy query probe should evaluate");

    assert_eq!(
        vm.eval("String(globalThis.__policyQuery)").unwrap(),
        "denied,prompt,prompt,prompt"
    );
}

#[test]
fn permissions_query_defaults_to_prompt_without_a_directive() {
    let url = "https://permissions-policy-default.test/";
    let mut vm = new_storage_test_vm(url);
    vm.set_main_navigation_policy_container(policy_container_with_permissions_policy(
        url,
        "geolocation=()",
    ));

    vm.eval(
        r#"
      (() => {
        globalThis.__defaultQuery = "pending";
        navigator.permissions.query({ name: "camera" }).then(status => {
          globalThis.__defaultQuery = status.state;
        });
      })()
    "#,
    )
    .expect("permissions policy default probe should evaluate");

    assert_eq!(
        vm.eval("String(globalThis.__defaultQuery)").unwrap(),
        "prompt"
    );
}
