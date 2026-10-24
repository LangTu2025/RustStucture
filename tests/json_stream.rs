use rust_water_structure::json_stream;
use serde_json::Value;

#[test]
fn array_visits_values_before_reading_the_remainder() {
    let mut values = Vec::new();
    let result = json_stream::visit_array::<_, u64, _>(
        b"[1,9007199254740993,invalid]".as_slice(),
        |value| {
            values.push(value);
            Ok(())
        },
    );
    assert!(result.is_err());
    assert_eq!(values, [1, 9007199254740993]);
}

#[test]
fn object_skips_unselected_nested_values() {
    let input = br#"{"large":[{"nested":[1,true,null,"x"]}],"chosen":9223372036854775807}"#;
    let mut selected = Vec::new();
    json_stream::visit_object(
        input.as_slice(),
        |key| key == "chosen",
        |key, value| {
            selected.push((key, value));
            Ok(())
        },
    )
    .unwrap();
    assert_eq!(selected, [("chosen".to_owned(), Value::from(i64::MAX))]);
}

#[test]
fn skip_validates_syntax_and_rejects_trailing_input() {
    json_stream::skip_value(br#"{"x":[1,{"y":"\u4e2d"}]}"#.as_slice()).unwrap();
    assert!(json_stream::skip_value(b"[1,]".as_slice()).is_err());
    assert!(json_stream::skip_value(b"{} false".as_slice()).is_err());
}

#[test]
fn callback_failure_stops_array_processing() {
    let mut seen = 0;
    let result = json_stream::visit_array::<_, u32, _>(b"[1,2,3]".as_slice(), |_| {
        seen += 1;
        Err(rust_water_structure::Error::Cancelled)
    });
    assert!(result.is_err());
    assert_eq!(seen, 1);
}
