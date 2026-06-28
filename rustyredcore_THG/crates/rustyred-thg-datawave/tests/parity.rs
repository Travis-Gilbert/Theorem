//! Ingest parity lane (the spec's "Next Build Cut"): run DATAWAVE's own checked-in
//! ingest fixtures through these contracts and assert the normalized field-facts,
//! index policy, and flatten structure against DATAWAVE's asserted outputs.
//!
//! Oracle: `NationalSecurityAgency/datawave` @ `integration`.
//! - CSV: `warehouse/ingest-csv/.../NormalizedContentInterfaceTest` + the
//!   `my-nci.csv` input and `norm-content-interface.xml` config. The field ->
//!   normalized-value pairs and index policy are literally asserted there.
//! - JSON: `warehouse/ingest-json/.../JsonObjectFlattenerImplTest`. The flattened
//!   key/value counts (25 keys / 29 values, DATE x3) are asserted there.
//!
//! These tests fix this crate's normalizers and flattener against the reference,
//! so the field-facts are checked against real expected outputs, not asserted into
//! existence. The same parity receipts can feed the learned-scorer training stream
//! the reconstruction lane feeds.

use rustyred_thg_datawave::{
    CsvHelper, FieldConfig, FieldType, IndexPolicy, IngestHelper, JsonHelper, NormalizedField,
    RawRecord,
};
use std::collections::BTreeMap;

fn by_field(fields: &[NormalizedField]) -> BTreeMap<&str, &NormalizedField> {
    fields.iter().map(|f| (f.field.as_str(), f)).collect()
}

/// DATAWAVE `my-nci.csv` line 1 + `norm-content-interface.xml` config.
#[test]
fn csv_my_nci_matches_datawave_normalized_facts() {
    let indexed = IndexPolicy::INDEXED.with_reverse();
    let config = FieldConfig::new()
        .with_field("HEADER_DATE", FieldType::Date, indexed)
        .with_field("HEADER_ID", FieldType::LcText, indexed) // StringType = LcNoDiacritics
        .with_field("HEADER_NUMBER", FieldType::Number, indexed)
        // TEXT_1 / TEXT_2 are on the index disallowlist: stored, not indexed.
        .with_field("HEADER_TEXT_1", FieldType::LcText, IndexPolicy::NONE)
        .with_field("HEADER_TEXT_2", FieldType::LcText, IndexPolicy::NONE)
        .with_field("DATE_FIELD", FieldType::Date, indexed);

    let helper = CsvHelper::new(
        "mycsv",
        ["HEADER_DATE", "HEADER_ID", "HEADER_NUMBER", "HEADER_TEXT_1", "HEADER_TEXT_2"],
        config,
    )
    .with_extra_fields();

    let line = "2024-02-29 12:01:47,header_one,111,text one-one,text two-one,DATE_FIELD=2024-02-29 12:01:47";
    let record = RawRecord::text("mycsv", line, 0);
    let fields = helper.event_fields(&record).unwrap();
    let map = by_field(&fields);

    // field -> (raw, normalized) asserted by DATAWAVE.
    let expected = [
        ("HEADER_DATE", "2024-02-29 12:01:47", "2024-02-29T12:01:47.000Z"),
        ("HEADER_ID", "header_one", "header_one"),
        ("HEADER_NUMBER", "111", "+cE1.11"),
        ("HEADER_TEXT_1", "text one-one", "text one-one"),
        ("HEADER_TEXT_2", "text two-one", "text two-one"),
        ("DATE_FIELD", "2024-02-29 12:01:47", "2024-02-29T12:01:47.000Z"),
    ];
    for (field, raw, normalized) in expected {
        let fact = map.get(field).unwrap_or_else(|| panic!("missing field {field}"));
        assert_eq!(fact.raw_value, raw, "{field} raw");
        assert_eq!(fact.normalized, normalized, "{field} normalized");
    }
    assert_eq!(fields.len(), 6);

    // Index policy: the two TEXT fields are excluded; the rest are indexed.
    assert!(map["HEADER_DATE"].policy.indexed && map["HEADER_DATE"].policy.reverse_indexed);
    assert!(map["HEADER_NUMBER"].policy.indexed);
    assert!(!map["HEADER_TEXT_1"].policy.indexed && !map["HEADER_TEXT_1"].policy.reverse_indexed);
    assert!(!map["HEADER_TEXT_2"].policy.indexed);
}

const FLATTENER_JSON: &str = r#"
{
  "rootobject":
  {
    "sTrInG1": "string1 text",
    "boolean": true,
    "number": 101,
    "string2": "string2 text",
    "number2": "20000",
    "date" : [ "2017-01-01T01:01:01Z", "2017-02-01T02:02:01Z", "2017-03-01T03:03:03Z" ],
    "randomobject":
    {
      "boolean": false,
      "number": "150",
      "string": "horse"
    },
    "properties":
    {
      "array":
      [
        { "name": "P1Name", "value": "1", "description": "Description for P1Name" },
        { "name": "P2Name", "value": "Two", "description": "Description for P2Name" },
        [ { "name": "InnerPName1", "value": "InnerPValue1" }, { "name": "InnerPName2", "value": "InnerPValue2" } ]
      ]
    }
  },
  "rootdate" : "2017-01-04T01:00:00Z",
  "rootid" : "ID00000000004",
  "rootnumber" : 40,
  "rootarray" : [ "ITEM1", false, 7, { "more" : "nested", "stuff" : "to deal with" } ]
}
"#;

/// DATAWAVE `JsonObjectFlattenerImplTest`, NORMAL mode, "." delimiter, upper-cased
/// keys, array index on object members: 25 distinct keys, 29 values, DATE x3.
#[test]
fn json_flattener_matches_datawave_normal_mode_counts() {
    let value: serde_json::Value = serde_json::from_str(FLATTENER_JSON).unwrap();
    let helper = JsonHelper::new("doc", FieldConfig::new()).with_uppercase_keys();
    let record = RawRecord::json("doc", value, 0);
    let pairs = helper.extract(&record).unwrap();

    let total_values = pairs.len();
    let distinct_keys: std::collections::BTreeSet<&str> =
        pairs.iter().map(|(k, _)| k.as_str()).collect();
    let count = |key: &str| pairs.iter().filter(|(k, _)| k == key).count();

    assert_eq!(distinct_keys.len(), 25, "distinct flattened keys");
    assert_eq!(total_values, 29, "total flattened values");
    assert_eq!(count("ROOTOBJECT.DATE"), 3, "DATE is multi-valued x3");
    assert_eq!(count("ROOTARRAY"), 3, "array primitives share ROOTARRAY x3");

    // Spot-check specific keys, including the nested inner-array leaves.
    let has = |key: &str, val: &str| pairs.iter().any(|(k, v)| k == key && v == val);
    assert!(has("ROOTOBJECT.STRING1", "string1 text"));
    assert!(has("ROOTOBJECT.RANDOMOBJECT.STRING", "horse"));
    assert!(has("ROOTOBJECT.PROPERTIES.ARRAY.0.NAME", "P1Name"));
    assert!(has("ROOTOBJECT.PROPERTIES.ARRAY.2.0.NAME", "InnerPName1"));
    assert!(has("ROOTOBJECT.PROPERTIES.ARRAY.2.1.VALUE", "InnerPValue2"));
    assert!(has("ROOTARRAY.3.MORE", "nested"));
    // Index NOT appended for array primitives.
    assert!(!distinct_keys.contains("ROOTARRAY.0"));
}
