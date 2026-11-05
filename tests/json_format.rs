use semver_resolver::Json;

#[test]
fn parses_scalars_and_containers() {
    let doc = Json::parse(r#" {"a": [1, -2.5e2, true, false, null], "b": {"c": "d"}, "e": []} "#)
        .unwrap();
    let Some(Json::Array(items)) = doc.get("a") else { panic!("expected an array") };
    assert_eq!(items[0].as_f64(), Some(1.0));
    assert_eq!(items[1].as_f64(), Some(-250.0));
    assert_eq!(items[2], Json::Bool(true));
    assert_eq!(items[3], Json::Bool(false));
    assert_eq!(items[4], Json::Null);
    assert_eq!(doc.get("b").and_then(|b| b.get("c")).and_then(Json::as_str), Some("d"));
    assert_eq!(doc.get("e"), Some(&Json::Array(vec![])));
    assert_eq!(doc.get("missing"), None);
    assert_eq!(items[0].get("anything"), None);
}

#[test]
fn keeps_object_key_order() {
    let doc = Json::parse(r#"{"z": 1, "a": 2, "m": 3}"#).unwrap();
    let keys: Vec<&str> = doc.as_object().unwrap().iter().map(|(k, _)| k.as_str()).collect();
    assert_eq!(keys, ["z", "a", "m"]);
}

#[test]
fn decodes_every_escape() {
    let doc = Json::parse(r#""\"\\\/\b\f\n\r\t\u0041\u00e9\ud83d\ude00""#).unwrap();
    assert_eq!(doc.as_str(), Some("\"\\/\u{8}\u{c}\n\r\tAé😀"));
}

#[test]
fn rejects_malformed_documents() {
    for text in [
        "",
        "   ",
        "{",
        "[",
        "[1,",
        "[1,]",
        "[1 2]",
        "{\"a\"}",
        "{\"a\":}",
        "{\"a\":1,}",
        "{a:1}",
        "{\"a\":1 \"b\":2}",
        "tru",
        "nul",
        "True",
        "01",
        "-",
        "1.",
        ".5",
        "1e",
        "1e+",
        "+1",
        "1e999",
        "\"abc",
        "\"a\nb\"",
        "\"\\x\"",
        "\"\\u12\"",
        "\"\\ud83d\"",
        "\"\\ud83dx\"",
        "\"\\ud83d\\u0041\"",
        "\"\\ude00\"",
        "1 2",
        "{} {}",
        "nullx",
    ] {
        assert!(Json::parse(text).is_err(), "should reject {text:?}");
    }
}

#[test]
fn reports_line_and_column() {
    let err = Json::parse("{\n  \"a\": 1,\n  \"b\" 2\n}").unwrap_err();
    assert_eq!((err.line, err.column), (3, 7));
    assert!(err.to_string().starts_with("line 3, column 7:"), "{err}");
}

#[test]
fn duplicate_keys_are_rejected_at_the_second_key() {
    let err = Json::parse("{\"a\": 1,\n \"a\": 2}").unwrap_err();
    assert_eq!((err.line, err.column), (2, 2));
    assert!(err.message.contains("duplicate key `a`"));
}

#[test]
fn nesting_limit_protects_the_stack() {
    let deep = format!("{}{}", "[".repeat(100_000), "]".repeat(100_000));
    let err = Json::parse(&deep).unwrap_err();
    assert!(err.message.contains("too deep"));

    let fine = format!("{}{}", "[".repeat(128), "]".repeat(128));
    assert!(Json::parse(&fine).is_ok());
    let too_many = format!("{}{}", "[".repeat(129), "]".repeat(129));
    assert!(Json::parse(&too_many).is_err());
}

#[test]
fn output_escapes_and_round_trips() {
    let text = r#"{"quote\"":"line\nbreak\u0001","n":[1,2.5,-3,1e21,0.001],"t":true,"none":null,"e":{}}"#;
    let doc = Json::parse(text).unwrap();
    let written = doc.pretty();
    assert!(written.contains(r#""quote\"": "line\nbreak\u0001""#), "{written}");
    assert_eq!(Json::parse(&written).unwrap(), doc);
}

#[test]
fn whole_numbers_print_without_a_fraction() {
    assert_eq!(Json::Number(3.0).pretty(), "3\n");
    assert_eq!(Json::Number(-0.5).pretty(), "-0.5\n");
    assert_eq!(Json::Number(f64::NAN).pretty(), "null\n");
    assert_eq!(Json::Number(f64::INFINITY).pretty(), "null\n");
}

#[test]
fn pretty_form_is_indented_and_stable() {
    let doc = Json::parse(r#"{"a":[1,{"b":[]}],"c":{},"d":"x"}"#).unwrap();
    let expected = "{\n  \"a\": [\n    1,\n    {\n      \"b\": []\n    }\n  ],\n  \"c\": {},\n  \"d\": \"x\"\n}\n";
    assert_eq!(doc.pretty(), expected);
    assert_eq!(Json::parse(&doc.pretty()).unwrap(), doc);
}

#[test]
fn non_ascii_text_passes_through_unescaped() {
    let doc = Json::string("naïve ☕");
    assert_eq!(doc.pretty(), "\"naïve ☕\"\n");
    assert_eq!(Json::parse(&doc.pretty()).unwrap(), doc);
}
