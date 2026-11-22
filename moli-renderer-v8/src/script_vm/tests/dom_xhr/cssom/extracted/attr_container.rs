use super::*;

#[test]
fn container_attr_queries_refresh_after_referenced_attribute_mutations() {
    let mut vm = new_storage_html_test_vm("https://container-attr-invalidation.test/page.html");
    let result = vm
        .eval(
            r#"
const style = document.createElement('style');
style.textContent = '@container style(1px < attr(data-length type(<length>), 0px) < 10px) { .target { --hit: true; } }';
document.head.append(style);

const container = document.createElement('main');
const target = document.createElement('div');
target.className = 'target';
container.append(target);
document.body.append(container);

const value = () => getComputedStyle(target).getPropertyValue('--hit').trim();
const values = [value()];
for (const length of ['5px', '20px', '5px']) {
  container.setAttribute('data-length', length);
  values.push(value());
}
container.removeAttribute('data-length');
values.push(value());
JSON.stringify(values);
"#,
        )
        .expect("container attr invalidation probe should evaluate");
    assert_eq!(result, r#"["","true","","true",""]"#);
}
