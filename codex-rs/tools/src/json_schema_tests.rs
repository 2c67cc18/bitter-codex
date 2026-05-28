use super::AdditionalProperties;
use super::JsonSchema;
use super::JsonSchemaPrimitiveType;
use super::JsonSchemaType;
use super::parse_tool_input_schema;
use pretty_assertions::assert_eq;
use std::collections::BTreeMap;

#[test]
fn parse_tool_input_schema_coerces_boolean_schemas() {
    let schema = parse_tool_input_schema(&serde_json::json!(true)).expect("parse schema");

    assert_eq!(schema, JsonSchema::string(None));
}

#[test]
fn parse_tool_input_schema_infers_object_shape_and_defaults_properties() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "properties": {
            "query": {"description": "search query"}
        }
    }))
    .expect("parse schema");

    assert_eq!(
        schema,
        JsonSchema::object(
            BTreeMap::from([("query".to_string(), JsonSchema::default())]),
            None,
            None
        )
    );
}

#[test]
fn parse_tool_input_schema_coerces_unrecognized_object_schema_to_empty_schema() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "description": "Ticket identifier",
        "title": "Ticket ID"
    }))
    .expect("parse schema");

    assert_eq!(schema, JsonSchema::default());
}

#[test]
fn parse_tool_input_schema_preserves_integer_and_defaults_array_items() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "type": "object",
        "properties": {
            "page": {"type": "integer"},
            "tags": {"type": "array"}
        }
    }))
    .expect("parse schema");

    assert_eq!(
        schema,
        JsonSchema::object(
            BTreeMap::from([
                ("page".to_string(), JsonSchema::integer(None),),
                (
                    "tags".to_string(),
                    JsonSchema::array(JsonSchema::string(None), None,)
                ),
            ]),
            None,
            None
        )
    );
}

#[test]
fn parse_tool_input_schema_sanitizes_additional_properties_schema() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "type": "object",
        "additionalProperties": {
            "required": ["value"],
            "properties": {
                "value": {"anyOf": [{"type": "string"}, {"type": "number"}]}
            }
        }
    }))
    .expect("parse schema");

    assert_eq!(
        schema,
        JsonSchema::object(
            BTreeMap::new(),
            None,
            Some(AdditionalProperties::Schema(Box::new(JsonSchema::object(
                BTreeMap::from([(
                    "value".to_string(),
                    JsonSchema::any_of(
                        vec![JsonSchema::string(None), JsonSchema::number(None),],
                        None,
                    ),
                )]),
                Some(vec!["value".to_string()]),
                None,
            ))))
        )
    );
}

#[test]
fn parse_tool_input_schema_infers_object_shape_from_boolean_additional_properties_only() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "additionalProperties": false
    }))
    .expect("parse schema");

    assert_eq!(
        schema,
        JsonSchema::object(BTreeMap::new(), None, Some(false.into()))
    );
}

#[test]
fn parse_tool_input_schema_infers_number_from_numeric_keywords() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "minimum": 1
    }))
    .expect("parse schema");

    assert_eq!(schema, JsonSchema::number(None));
}

#[test]
fn parse_tool_input_schema_infers_number_from_multiple_of() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "multipleOf": 5
    }))
    .expect("parse schema");

    assert_eq!(schema, JsonSchema::number(None));
}

#[test]
fn parse_tool_input_schema_infers_string_from_enum_const_and_format_keywords() {
    let enum_schema = parse_tool_input_schema(&serde_json::json!({
        "enum": ["fast", "safe"]
    }))
    .expect("parse enum schema");
    let const_schema = parse_tool_input_schema(&serde_json::json!({
        "const": "file"
    }))
    .expect("parse const schema");
    let format_schema = parse_tool_input_schema(&serde_json::json!({
        "format": "date-time"
    }))
    .expect("parse format schema");

    assert_eq!(
        enum_schema,
        JsonSchema::string_enum(
            vec![serde_json::json!("fast"), serde_json::json!("safe")],
            None,
        )
    );
    assert_eq!(
        const_schema,
        JsonSchema::string_enum(vec![serde_json::json!("file")], None)
    );
    assert_eq!(format_schema, JsonSchema::string(None));
}

#[test]
fn parse_tool_input_schema_preserves_empty_schema() {
    let schema = parse_tool_input_schema(&serde_json::json!({})).expect("parse schema");

    assert_eq!(schema, JsonSchema::default());
}

#[test]
fn parse_tool_input_schema_preserves_nested_empty_schema() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "type": "object",
        "properties": {
            "metadata": {
                "properties": {
                    "extra": {}
                }
            }
        }
    }))
    .expect("parse schema");

    assert_eq!(
        schema,
        JsonSchema::object(
            BTreeMap::from([(
                "metadata".to_string(),
                JsonSchema::object(
                    BTreeMap::from([("extra".to_string(), JsonSchema::default())]),
                    None,
                    None,
                )
            )]),
            None,
            None,
        )
    );
}

#[test]
fn parse_tool_input_schema_infers_array_from_prefix_items() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "prefixItems": [
            {"type": "string"}
        ]
    }))
    .expect("parse schema");

    assert_eq!(schema, JsonSchema::array(JsonSchema::string(None), None,));
}

#[test]
fn parse_tool_input_schema_preserves_boolean_additional_properties_on_inferred_object() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "type": "object",
        "properties": {
            "metadata": {
                "additionalProperties": true
            }
        }
    }))
    .expect("parse schema");

    assert_eq!(
        schema,
        JsonSchema::object(
            BTreeMap::from([(
                "metadata".to_string(),
                JsonSchema::object(BTreeMap::new(), None, Some(true.into())),
            )]),
            None,
            None
        )
    );
}

#[test]
fn parse_tool_input_schema_infers_object_shape_from_schema_additional_properties_only() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "additionalProperties": {
            "type": "string"
        }
    }))
    .expect("parse schema");

    assert_eq!(
        schema,
        JsonSchema::object(BTreeMap::new(), None, Some(JsonSchema::string(None).into()))
    );
}

#[test]
fn parse_tool_input_schema_rewrites_const_to_single_value_enum() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "const": "tagged"
    }))
    .expect("parse schema");

    assert_eq!(
        schema,
        JsonSchema::string_enum(vec![serde_json::json!("tagged")], None)
    );
}

#[test]
fn parse_tool_input_schema_rejects_singleton_null_type() {
    let err = parse_tool_input_schema(&serde_json::json!({
        "type": "null"
    }))
    .expect_err("singleton null should be rejected");

    assert!(
        err.to_string()
            .contains("tool input schema must not be a singleton null type"),
        "unexpected error: {err}"
    );
}

#[test]
fn parse_tool_input_schema_fills_default_properties_for_nullable_object_union() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "type": ["object", "null"]
    }))
    .expect("parse schema");

    assert_eq!(
        schema,
        JsonSchema {
            schema_type: Some(JsonSchemaType::Multiple(vec![
                JsonSchemaPrimitiveType::Object,
                JsonSchemaPrimitiveType::Null,
            ])),
            properties: Some(BTreeMap::new()),
            ..Default::default()
        }
    );
}

#[test]
fn parse_tool_input_schema_fills_default_items_for_nullable_array_union() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "type": ["array", "null"]
    }))
    .expect("parse schema");

    assert_eq!(
        schema,
        JsonSchema {
            schema_type: Some(JsonSchemaType::Multiple(vec![
                JsonSchemaPrimitiveType::Array,
                JsonSchemaPrimitiveType::Null,
            ])),
            items: Some(Box::new(JsonSchema::string(None))),
            ..Default::default()
        }
    );
}

#[test]
fn parse_tool_input_schema_preserves_nested_nullable_any_of_shape() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "type": "object",
        "properties": {
            "open": {
                "anyOf": [
                    {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "ref_id": {"type": "string"},
                                "lineno": {"anyOf": [{"type": "integer"}, {"type": "null"}]}
                            },
                            "required": ["ref_id"],
                            "additionalProperties": false
                        }
                    },
                    {"type": "null"}
                ]
            }
        }
    }))
    .expect("parse schema");

    assert_eq!(
        schema,
        JsonSchema::object(
            BTreeMap::from([(
                "open".to_string(),
                JsonSchema::any_of(
                    vec![
                        JsonSchema::array(
                            JsonSchema::object(
                                BTreeMap::from([
                                    (
                                        "lineno".to_string(),
                                        JsonSchema::any_of(
                                            vec![JsonSchema::integer(None), JsonSchema::null(None),],
                                            None,
                                        ),
                                    ),
                                    ("ref_id".to_string(), JsonSchema::string(None),),
                                ]),
                                Some(vec!["ref_id".to_string()]),
                                Some(false.into()),
                            ),
                            None,
                        ),
                        JsonSchema::null(None),
                    ],
                    None,
                ),
            ),]),
            None,
            None
        )
    );
}

#[test]
fn parse_tool_input_schema_preserves_nested_nullable_type_union() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "type": "object",
        "properties": {
            "nickname": {
                "type": ["string", "null"],
                "description": "Optional nickname"
            }
        },
        "required": ["nickname"],
        "additionalProperties": false
    }))
    .expect("parse schema");

    assert_eq!(
        schema,
        JsonSchema::object(
            BTreeMap::from([(
                "nickname".to_string(),
                JsonSchema {
                    schema_type: Some(JsonSchemaType::Multiple(vec![
                        JsonSchemaPrimitiveType::String,
                        JsonSchemaPrimitiveType::Null,
                    ])),
                    description: Some("Optional nickname".to_string()),
                    ..Default::default()
                },
            )]),
            Some(vec!["nickname".to_string()]),
            Some(false.into()),
        )
    );
}

#[test]
fn parse_tool_input_schema_preserves_nested_any_of_property() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "type": "object",
        "properties": {
            "query": {
                "anyOf": [
                    { "type": "string" },
                    { "type": "number" }
                ]
            }
        }
    }))
    .expect("parse schema");

    assert_eq!(
        schema,
        JsonSchema::object(
            BTreeMap::from([(
                "query".to_string(),
                JsonSchema::any_of(
                    vec![JsonSchema::string(None), JsonSchema::number(None),],
                    None,
                ),
            )]),
            None,
            None
        )
    );
}

#[test]
fn parse_tool_input_schema_preserves_type_unions_without_rewriting_to_any_of() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "type": ["string", "null"],
        "description": "optional string"
    }))
    .expect("parse schema");

    assert_eq!(
        schema,
        JsonSchema {
            schema_type: Some(JsonSchemaType::Multiple(vec![
                JsonSchemaPrimitiveType::String,
                JsonSchemaPrimitiveType::Null,
            ])),
            description: Some("optional string".to_string()),
            ..Default::default()
        }
    );
}

#[test]
fn parse_tool_input_schema_preserves_explicit_enum_type_union() {
    let schema = super::parse_tool_input_schema(&serde_json::json!({
        "type": ["string", "null"],
        "enum": ["short", "medium", "long"],
        "description": "optional response length"
    }))
    .expect("parse schema");

    assert_eq!(
        schema,
        JsonSchema {
            schema_type: Some(JsonSchemaType::Multiple(vec![
                JsonSchemaPrimitiveType::String,
                JsonSchemaPrimitiveType::Null,
            ])),
            description: Some("optional response length".to_string()),
            enum_values: Some(vec![
                serde_json::json!("short"),
                serde_json::json!("medium"),
                serde_json::json!("long"),
            ]),
            ..Default::default()
        }
    );
}

fn many_string_properties(count: usize) -> serde_json::Map<String, serde_json::Value> {
    (0..count)
        .map(|index| {
            (
                format!("field_{index:03}"),
                serde_json::json!({ "type": "string" }),
            )
        })
        .collect()
}

#[test]
fn parse_large_tool_input_schema_stops_after_descriptions_when_under_budget() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "type": "object",
        "description": "x".repeat(4_500),
        "properties": {
            "metadata": {
                "$ref": "#/$defs/metadata"
            }
        },
        "$defs": {
            "metadata": {
                "type": "string",
                "description": "Metadata value"
            }
        }
    }))
    .expect("parse schema");

    assert_eq!(
        serde_json::to_value(schema).expect("serialize schema"),
        serde_json::json!({
            "type": "object",
            "properties": {
                "metadata": {
                    "$ref": "#/$defs/metadata"
                }
            },
            "$defs": {
                "metadata": {
                    "type": "string"
                }
            }
        })
    );
}

#[test]
fn parse_large_tool_input_schema_ignores_dropped_metadata_for_budget() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "type": "object",
        "properties": {
            "event": {
                "type": "object",
                "title": "Calendar event",
                "properties": {
                    "recurrence": {
                        "type": "object",
                        "examples": [
                            {
                                "payload": "x".repeat(4_500)
                            }
                        ],
                        "properties": {
                            "pattern": {
                                "type": "string",
                                "title": "Recurrence pattern"
                            }
                        }
                    }
                }
            }
        }
    }))
    .expect("parse schema");

    assert_eq!(
        serde_json::to_value(schema).expect("serialize schema"),
        serde_json::json!({
            "type": "object",
            "properties": {
                "event": {
                    "type": "object",
                    "properties": {
                        "recurrence": {
                            "type": "object",
                            "properties": {
                                "pattern": {
                                    "type": "string"
                                }
                            }
                        }
                    }
                }
            }
        })
    );
}

#[test]
fn parse_large_tool_input_schema_stops_after_dropping_root_definitions_when_under_budget() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "type": "object",
        "description": "x".repeat(4_500),
        "properties": {
            "event": {
                "type": "object",
                "description": "Calendar event",
                "properties": {
                    "recurrence": {
                        "type": "object",
                        "description": "Recurrence settings",
                        "properties": {
                            "pattern": {
                                "type": "string",
                                "description": "Recurrence pattern"
                            }
                        }
                    }
                }
            },
            "metadata": {
                "$ref": "#/$defs/metadata"
            }
        },
        "$defs": {
            "metadata": {
                "type": "object",
                "description": "metadata object",
                "properties": many_string_properties( 300)
            }
        }
    }))
    .expect("parse schema");

    assert_eq!(
        serde_json::to_value(schema).expect("serialize schema"),
        serde_json::json!({
            "type": "object",
            "properties": {
                "event": {
                    "type": "object",
                    "properties": {
                        "recurrence": {
                            "type": "object",
                            "properties": {
                                "pattern": {
                                    "type": "string"
                                }
                            }
                        }
                    }
                },
                "metadata": {}
            }
        })
    );
}

#[test]
fn parse_large_tool_input_schema_strips_descriptions_without_removing_description_property() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "type": "object",
        "description": "x".repeat(4_500),
        "properties": {
            "description": {
                "type": "string",
                "description": "User-facing description value"
            },
            "metadata": {
                "type": "object",
                "description": "Metadata object",
                "properties": {
                    "label": {
                        "type": "string",
                        "description": "Metadata label"
                    }
                }
            },
            "tags": {
                "type": "array",
                "description": "Tag list",
                "items": {
                    "type": "string",
                    "description": "Tag value"
                }
            },
            "extras": {
                "type": "object",
                "additionalProperties": {
                    "type": "string",
                    "description": "Extra value"
                }
            },
            "choice": {
                "description": "Choice value",
                "anyOf": [
                    {
                        "type": "string",
                        "description": "String choice"
                    },
                    {
                        "type": "number",
                        "description": "Number choice"
                    }
                ]
            }
        }
    }))
    .expect("parse schema");

    assert_eq!(
        serde_json::to_value(schema).expect("serialize schema"),
        serde_json::json!({
            "type": "object",
            "properties": {
                "choice": {
                    "anyOf": [
                        {
                            "type": "string"
                        },
                        {
                            "type": "number"
                        }
                    ]
                },
                "description": {
                    "type": "string"
                },
                "extras": {
                    "type": "object",
                    "properties": {},
                    "additionalProperties": {
                        "type": "string"
                    }
                },
                "metadata": {
                    "type": "object",
                    "properties": {
                        "label": {
                            "type": "string"
                        }
                    }
                },
                "tags": {
                    "type": "array",
                    "items": {
                        "type": "string"
                    }
                }
            }
        })
    );
}

#[test]
fn parse_large_tool_input_schema_preserves_object_enum_literal_descriptions() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "type": "object",
        "description": "x".repeat(4_500),
        "properties": {
            "choice": {
                "enum": [
                    {
                        "description": "first literal",
                        "id": 1
                    },
                    {
                        "description": "second literal",
                        "id": 2
                    }
                ]
            }
        }
    }))
    .expect("parse schema");

    assert_eq!(
        serde_json::to_value(schema).expect("serialize schema"),
        serde_json::json!({
            "type": "object",
            "properties": {
                "choice": {
                    "type": "string",
                    "enum": [
                        {
                            "description": "first literal",
                            "id": 1
                        },
                        {
                            "description": "second literal",
                            "id": 2
                        }
                    ]
                }
            }
        })
    );
}

#[test]
fn collapse_deep_schema_objects_traverses_schema_children() {
    let mut schema = serde_json::json!({
        "type": "object",
        "properties": {
            "object_parent": {
                "type": "object",
                "properties": {
                    "complex": {
                        "type": "object",
                        "properties": {
                            "leaf": { "type": "string" }
                        }
                    },
                    "scalar": {
                        "type": "string"
                    }
                }
            },
            "array_parent": {
                "type": "array",
                "items": {
                    "type": "object",
                    "properties": {
                        "leaf": { "type": "string" }
                    }
                }
            },
            "map_parent": {
                "type": "object",
                "additionalProperties": {
                    "type": "object",
                    "properties": {
                        "leaf": { "type": "string" }
                    }
                }
            },
            "union_parent": {
                "anyOf": [
                    {
                        "type": "object",
                        "properties": {
                            "leaf": { "type": "string" }
                        }
                    },
                    { "type": "string" }
                ]
            }
        }
    });

    super::collapse_deep_schema_objects(&mut schema, 0);

    assert_eq!(
        schema,
        serde_json::json!({
            "type": "object",
            "properties": {
                "object_parent": {
                    "type": "object",
                    "properties": {
                        "complex": {},
                        "scalar": {
                            "type": "string"
                        }
                    }
                },
                "array_parent": {
                    "type": "array",
                    "items": {}
                },
                "map_parent": {
                    "type": "object",
                    "additionalProperties": {}
                },
                "union_parent": {
                    "anyOf": [
                        {},
                        { "type": "string" }
                    ]
                }
            }
        })
    );
}

#[test]
fn parse_tool_input_schema_preserves_string_enum_constraints() {
    let schema = super::parse_tool_input_schema(&serde_json::json!({
        "type": "object",
        "properties": {
            "response_length": {
                "type": "enum",
                "enum": ["short", "medium", "long"]
            },
            "kind": {
                "type": "const",
                "const": "tagged"
            },
            "scope": {
                "type": "enum",
                "enum": ["one", "two"]
            }
        }
    }))
    .expect("parse schema");

    assert_eq!(
        schema,
        JsonSchema::object(
            BTreeMap::from([
                (
                    "kind".to_string(),
                    JsonSchema::string_enum(vec![serde_json::json!("tagged")], None,),
                ),
                (
                    "response_length".to_string(),
                    JsonSchema::string_enum(
                        vec![
                            serde_json::json!("short"),
                            serde_json::json!("medium"),
                            serde_json::json!("long"),
                        ],
                        None,
                    ),
                ),
                (
                    "scope".to_string(),
                    JsonSchema::string_enum(
                        vec![serde_json::json!("one"), serde_json::json!("two")],
                        None,
                    ),
                ),
            ]),
            None,
            None
        )
    );
}

#[test]
fn parse_tool_input_schema_preserves_refs_and_prunes_unreachable_defs() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "type": "object",
        "properties": {
            "user": {"$ref": "#/$defs/User"}
        },
        "$defs": {
            "User": {
                "type": "object",
                "properties": {
                    "name": {"type": "string"}
                }
            },
            "Unused": {"type": "string"}
        }
    }))
    .expect("parse schema");

    assert_eq!(
        schema,
        JsonSchema {
            schema_type: Some(JsonSchemaType::Single(JsonSchemaPrimitiveType::Object)),
            properties: Some(BTreeMap::from([(
                "user".to_string(),
                JsonSchema {
                    schema_ref: Some("#/$defs/User".to_string()),
                    ..Default::default()
                },
            )])),
            defs: Some(BTreeMap::from([(
                "User".to_string(),
                JsonSchema::object(
                    BTreeMap::from([("name".to_string(), JsonSchema::string(None),)]),
                    None,
                    None,
                ),
            )])),
            ..Default::default()
        }
    );
}

#[test]
fn parse_tool_input_schema_preserves_refs_from_properties_named_def_tables() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "type": "object",
        "properties": {
            "$defs": {"$ref": "#/$defs/User"}
        },
        "$defs": {
            "User": {"type": "string"},
            "Unused": {"type": "boolean"}
        }
    }))
    .expect("parse schema");

    assert_eq!(
        schema,
        JsonSchema {
            schema_type: Some(JsonSchemaType::Single(JsonSchemaPrimitiveType::Object)),
            properties: Some(BTreeMap::from([(
                "$defs".to_string(),
                JsonSchema {
                    schema_ref: Some("#/$defs/User".to_string()),
                    ..Default::default()
                },
            )])),
            defs: Some(BTreeMap::from([(
                "User".to_string(),
                JsonSchema::string(None),
            )])),
            ..Default::default()
        }
    );
}

#[test]
fn parse_tool_input_schema_collects_refs_from_schema_child_keywords() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "type": "object",
        "properties": {
            "items_holder": {
                "type": "array",
                "items": {"$ref": "#/$defs/Item"}
            },
            "map_holder": {
                "type": "object",
                "additionalProperties": {"$ref": "#/$defs/Extra"}
            },
            "choice": {
                "anyOf": [
                    {"$ref": "#/$defs/Choice"},
                    {"type": "string"}
                ]
            }
        },
        "$defs": {
            "Choice": {"type": "boolean"},
            "Extra": {"type": "number"},
            "Item": {"type": "string"},
            "Unused": {"type": "null"}
        }
    }))
    .expect("parse schema");

    assert_eq!(
        serde_json::to_value(schema).expect("serialize schema"),
        serde_json::json!({
            "type": "object",
            "properties": {
                "choice": {
                    "anyOf": [
                        {"$ref": "#/$defs/Choice"},
                        {"type": "string"}
                    ]
                },
                "items_holder": {
                    "type": "array",
                    "items": {"$ref": "#/$defs/Item"}
                },
                "map_holder": {
                    "type": "object",
                    "properties": {},
                    "additionalProperties": {"$ref": "#/$defs/Extra"}
                }
            },
            "$defs": {
                "Choice": {"type": "boolean"},
                "Extra": {"type": "number"},
                "Item": {"type": "string"}
            }
        })
    );
}

#[test]
fn parse_tool_input_schema_handles_cyclic_local_refs() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "type": "object",
        "properties": {
            "node": {"$ref": "#/$defs/Node"}
        },
        "$defs": {
            "Node": {
                "type": "object",
                "properties": {
                    "next": {"$ref": "#/$defs/Node"}
                }
            }
        }
    }))
    .expect("parse schema");

    assert_eq!(
        schema,
        JsonSchema {
            schema_type: Some(JsonSchemaType::Single(JsonSchemaPrimitiveType::Object)),
            properties: Some(BTreeMap::from([(
                "node".to_string(),
                JsonSchema {
                    schema_ref: Some("#/$defs/Node".to_string()),
                    ..Default::default()
                },
            )])),
            defs: Some(BTreeMap::from([(
                "Node".to_string(),
                JsonSchema::object(
                    BTreeMap::from([(
                        "next".to_string(),
                        JsonSchema {
                            schema_ref: Some("#/$defs/Node".to_string()),
                            ..Default::default()
                        },
                    )]),
                    None,
                    None,
                ),
            )])),
            ..Default::default()
        }
    );
}

#[test]
fn parse_tool_input_schema_preserves_definitions_table() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "type": "object",
        "properties": {
            "user": {"$ref": "#/definitions/User"}
        },
        "definitions": {
            "User": {
                "type": "object",
                "properties": {
                    "profile": {"$ref": "#/definitions/Profile"}
                }
            },
            "Profile": {
                "type": "object",
                "properties": {
                    "name": {"type": "string"}
                }
            },
            "Unused": {"type": "string"}
        }
    }))
    .expect("parse schema");

    assert_eq!(
        schema,
        JsonSchema {
            schema_type: Some(JsonSchemaType::Single(JsonSchemaPrimitiveType::Object)),
            properties: Some(BTreeMap::from([(
                "user".to_string(),
                JsonSchema {
                    schema_ref: Some("#/definitions/User".to_string()),
                    ..Default::default()
                },
            )])),
            definitions: Some(BTreeMap::from([
                (
                    "Profile".to_string(),
                    JsonSchema::object(
                        BTreeMap::from([("name".to_string(), JsonSchema::string(None),)]),
                        None,
                        None,
                    ),
                ),
                (
                    "User".to_string(),
                    JsonSchema::object(
                        BTreeMap::from([(
                            "profile".to_string(),
                            JsonSchema {
                                schema_ref: Some("#/definitions/Profile".to_string()),
                                ..Default::default()
                            },
                        )]),
                        None,
                        None,
                    ),
                ),
            ])),
            ..Default::default()
        }
    );
}

#[test]
fn parse_tool_input_schema_preserves_unresolved_and_external_refs() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "type": "object",
        "properties": {
            "missing": {"$ref": "#/$defs/Missing"},
            "remote": {"$ref": "https://example.com/schema.json"}
        },
        "$defs": {
            "Unused": {"type": "string"}
        }
    }))
    .expect("parse schema");

    assert_eq!(
        schema,
        JsonSchema {
            schema_type: Some(JsonSchemaType::Single(JsonSchemaPrimitiveType::Object)),
            properties: Some(BTreeMap::from([
                (
                    "missing".to_string(),
                    JsonSchema {
                        schema_ref: Some("#/$defs/Missing".to_string()),
                        ..Default::default()
                    },
                ),
                (
                    "remote".to_string(),
                    JsonSchema {
                        schema_ref: Some("https://example.com/schema.json".to_string()),
                        ..Default::default()
                    },
                ),
            ])),
            ..Default::default()
        }
    );
}

#[test]
fn parse_tool_input_schema_preserves_nested_defs_ref_parent() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "type": "object",
        "properties": {
            "name": {"$ref": "#/$defs/User/properties/name"}
        },
        "$defs": {
            "User": {
                "type": "object",
                "properties": {
                    "name": {"type": "string"}
                }
            },
            "name": {"type": "string"},
            "Unused": {"type": "boolean"}
        }
    }))
    .expect("parse schema");

    assert_eq!(
        schema,
        JsonSchema {
            schema_type: Some(JsonSchemaType::Single(JsonSchemaPrimitiveType::Object)),
            properties: Some(BTreeMap::from([(
                "name".to_string(),
                JsonSchema {
                    schema_ref: Some("#/$defs/User/properties/name".to_string()),
                    ..Default::default()
                },
            )])),
            defs: Some(BTreeMap::from([(
                "User".to_string(),
                JsonSchema::object(
                    BTreeMap::from([("name".to_string(), JsonSchema::string(None),)]),
                    None,
                    None,
                ),
            )])),
            ..Default::default()
        }
    );
}

#[test]
fn parse_tool_input_schema_preserves_percent_encoded_definition_refs() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "type": "object",
        "properties": {
            "user": {"$ref": "#/$defs/User%20Name"},
            "profile": {"$ref": "#/%24defs/Profile%7E0Name"}
        },
        "$defs": {
            "User Name": {"type": "string"},
            "Profile~Name": {"type": "string"},
            "Unused": {"type": "boolean"}
        }
    }))
    .expect("parse schema");

    assert_eq!(
        schema,
        JsonSchema {
            schema_type: Some(JsonSchemaType::Single(JsonSchemaPrimitiveType::Object)),
            properties: Some(BTreeMap::from([
                (
                    "profile".to_string(),
                    JsonSchema {
                        schema_ref: Some("#/%24defs/Profile%7E0Name".to_string()),
                        ..Default::default()
                    },
                ),
                (
                    "user".to_string(),
                    JsonSchema {
                        schema_ref: Some("#/$defs/User%20Name".to_string()),
                        ..Default::default()
                    },
                ),
            ])),
            defs: Some(BTreeMap::from([
                ("Profile~Name".to_string(), JsonSchema::string(None),),
                ("User Name".to_string(), JsonSchema::string(None),),
            ])),
            ..Default::default()
        }
    );
}

#[test]
fn parse_tool_input_schema_drops_malformed_definition_tables() {
    let schema = parse_tool_input_schema(&serde_json::json!({
        "type": "object",
        "properties": {
            "user": {"$ref": "#/$defs/User"}
        },
        "$defs": ["not", "an", "object"]
    }))
    .expect("parse schema");

    assert_eq!(
        schema,
        JsonSchema {
            schema_type: Some(JsonSchemaType::Single(JsonSchemaPrimitiveType::Object)),
            properties: Some(BTreeMap::from([(
                "user".to_string(),
                JsonSchema {
                    schema_ref: Some("#/$defs/User".to_string()),
                    ..Default::default()
                },
            )])),
            ..Default::default()
        }
    );
}
