use serde_json::Value;
use std::collections::BTreeSet;
use std::path::Path;

pub fn required_object<'a>(value: &'a Value, key: &str, path: &Path) -> &'a Value {
    value
        .get(key)
        .filter(|nested| nested.is_object())
        .unwrap_or_else(|| panic!("{} missing object field {key}", path.display()))
}

pub fn required_array<'a>(value: &'a Value, key: &str, path: &Path) -> &'a Vec<Value> {
    value
        .get(key)
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("{} missing array field {key}", path.display()))
}

pub fn optional_array<'a>(value: &'a Value, key: &str, path: &Path) -> &'a [Value] {
    match value.get(key) {
        Some(Value::Array(items)) => items,
        Some(_) => panic!("{} field {key} must be an array", path.display()),
        None => &[],
    }
}

pub fn required_str<'a>(value: &'a Value, key: &str, path: &Path) -> &'a str {
    value
        .get(key)
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("{} missing string field {key}", path.display()))
}

pub fn optional_str<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

pub fn required_u64(value: &Value, key: &str, path: &Path) -> u64 {
    value
        .get(key)
        .and_then(Value::as_u64)
        .unwrap_or_else(|| panic!("{} missing unsigned integer field {key}", path.display()))
}

pub fn assert_nonempty_str(value: &Value, key: &str, path: &Path) {
    let text = required_str(value, key, path);
    assert!(
        !text.trim().is_empty(),
        "{} field {key} must not be empty",
        path.display()
    );
}

pub fn assert_nonempty_string_array(value: &Value, key: &str, path: &Path) {
    let items = required_array(value, key, path);
    assert!(
        !items.is_empty(),
        "{} field {key} must not be empty",
        path.display()
    );
    for item in items {
        let Some(text) = item.as_str() else {
            panic!("{} field {key} must contain only strings", path.display());
        };
        assert!(
            !text.trim().is_empty(),
            "{} field {key} must not contain empty strings",
            path.display()
        );
    }
}

pub fn assert_string_array_contains_exactly(
    value: &Value,
    key: &str,
    expected: &[&str],
    path: &Path,
) {
    let actual: BTreeSet<_> = required_array(value, key, path)
        .iter()
        .map(|item| {
            item.as_str().unwrap_or_else(|| {
                panic!("{} field {key} must contain only strings", path.display())
            })
        })
        .collect();
    let expected: BTreeSet<_> = expected.iter().copied().collect();
    assert_eq!(actual, expected, "{} field {key} mismatch", path.display());
}

pub fn validate_optional_enum(value: &Value, key: &str, allowed: &BTreeSet<String>, path: &Path) {
    if let Some(actual) = optional_str(value, key) {
        assert_set_contains(allowed, actual, key, path);
    }
}

pub fn validate_string_array_enum(
    value: &Value,
    key: &str,
    allowed: &BTreeSet<String>,
    path: &Path,
) {
    for item in required_array(value, key, path) {
        let Some(actual) = item.as_str() else {
            panic!("{} field {key} must contain strings", path.display());
        };
        assert_set_contains(allowed, actual, key, path);
    }
}

pub fn assert_set_contains(allowed: &BTreeSet<String>, actual: &str, field: &str, path: &Path) {
    assert!(
        allowed.contains(actual),
        "{} field {field} has unknown value {actual}",
        path.display()
    );
}

pub fn assert_in(actual: &str, allowed: &[&str], field: &str, path: &Path) {
    assert!(
        allowed.contains(&actual),
        "{} field {field} has unknown value {actual}",
        path.display()
    );
}

pub fn assert_unique(set: &mut BTreeSet<String>, id: &str, kind: &str, path: &Path) {
    assert!(
        set.insert(id.to_string()),
        "{} duplicate {kind} id {id}",
        path.display()
    );
}
