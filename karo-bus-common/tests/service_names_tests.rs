use karo_bus_common::service_names::NamePattern;

#[test]

fn test_patterns() {
    assert!(NamePattern::from_string("com.*test.service").is_err());
    assert!(NamePattern::from_string("com.**test.service").is_err());

    assert!(NamePattern::from_string("com.**.service").is_ok());
    assert!(NamePattern::from_string("**.service").is_ok());
    assert!(NamePattern::from_string("com.*.service").is_ok());
    assert!(NamePattern::from_string("com.test.service").is_ok());
}

#[test]
fn test_full_name_patterns() {
    assert!(NamePattern::from_string("com..service").is_err());
    assert!(NamePattern::from_string("com.test.").is_err());
    assert!(NamePattern::from_string(".test.service").is_err());

    let pattern = NamePattern::from_string("com.test.service").unwrap();

    assert!(pattern.matches("com.*test.service").is_err());
    assert!(pattern.matches("com..service").is_err());
    assert!(pattern.matches("com.*test.").is_err());
    assert!(pattern.matches("com.test.service").unwrap());
}

#[test]
fn test_single_asterisk_name() {
    // Mathes
    assert!(NamePattern::from_string("*.test.service")
        .unwrap()
        .matches("com.test.service")
        .unwrap());
    assert!(NamePattern::from_string("com.*.service")
        .unwrap()
        .matches("com.test.service")
        .unwrap());
    assert!(NamePattern::from_string("com.test.*")
        .unwrap()
        .matches("com.test.service")
        .unwrap());
    assert!(NamePattern::from_string("*.*.service")
        .unwrap()
        .matches("com.test.service")
        .unwrap());
    assert!(NamePattern::from_string("com.*.*")
        .unwrap()
        .matches("com.test.service")
        .unwrap());
    assert!(NamePattern::from_string("*.test.*")
        .unwrap()
        .matches("com.test.service")
        .unwrap());
    assert!(NamePattern::from_string("*.*.*")
        .unwrap()
        .matches("com.test.service")
        .unwrap());

    // Not matches
    assert!(!NamePattern::from_string("*.test.service")
        .unwrap()
        .matches("test.service")
        .unwrap());
    assert!(!NamePattern::from_string("*.test.service")
        .unwrap()
        .matches("com.test")
        .unwrap());
    assert!(!NamePattern::from_string("com.*.service")
        .unwrap()
        .matches("com.service")
        .unwrap());
    assert!(!NamePattern::from_string("com.test.*")
        .unwrap()
        .matches("com.test")
        .unwrap());
    assert!(!NamePattern::from_string("*.*")
        .unwrap()
        .matches("com.test.service")
        .unwrap());
    assert!(!NamePattern::from_string("*.*.*.*")
        .unwrap()
        .matches("com.test.service")
        .unwrap());
}

#[test]
fn test_double_asterisk_name() {
    // Mathes
    assert!(NamePattern::from_string("**.test.service")
        .unwrap()
        .matches("com.test.service")
        .unwrap());
    assert!(NamePattern::from_string("com.**.service")
        .unwrap()
        .matches("com.test.service")
        .unwrap());
    assert!(NamePattern::from_string("com.test.**")
        .unwrap()
        .matches("com.test.service")
        .unwrap());

    assert!(NamePattern::from_string("**.service")
        .unwrap()
        .matches("com.test.service")
        .unwrap());
    assert!(NamePattern::from_string("com.**")
        .unwrap()
        .matches("com.test.service")
        .unwrap());
    assert!(NamePattern::from_string("**")
        .unwrap()
        .matches("com.test.service")
        .unwrap());

    // Not mathes
    assert!(!NamePattern::from_string("**.**")
        .unwrap()
        .matches("com")
        .unwrap());
}
