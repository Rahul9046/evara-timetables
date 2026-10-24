//! The JSON Schema each constraint type publishes, and the one place a stored `params` string
//! is parsed.
//!
//! # The schema is load-bearing
//!
//! ADR 0005: the schema *"validates writes **and** generates the configuration form"*, which
//! is why it lives beside the evaluator rather than in a form definition — and why *"a schema
//! that drifts from its evaluator is a real defect class"*. A test asserts every type has one,
//! that it parses as JSON, and that it accepts exactly the fields
//! [`ConstraintOptions`](evara_core::scheduling::ConstraintOptions) can hold.
//!
//! # Eight schemas are the same schema
//!
//! Eight of the nine implemented types take no parameters, so they publish the schema that
//! accepts an empty object and nothing else: `additionalProperties: false`. That is not a
//! placeholder — it is the statement that these rules have nothing to configure, and it makes
//! a row carrying parameters for one of them an error rather than a silently ignored field.
//!
//! `duration-contiguous` has the MVP's single parameter.

use evara_core::scheduling::{ConstraintError, ConstraintOptions, ConstraintType};

/// The schema for a type that takes nothing.
const NO_PARAMETERS: &str = r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "No parameters",
  "type": "object",
  "properties": {},
  "additionalProperties": false
}"#;

/// The schema for `duration-contiguous`.
const DURATION_CONTIGUOUS: &str = r#"{
  "$schema": "https://json-schema.org/draft/2020-12/schema",
  "title": "Contiguous duration",
  "type": "object",
  "properties": {
    "allowBreakSpan": {
      "type": "boolean",
      "default": false,
      "title": "Allow a double period to span a break",
      "description": "When false, a multi-period lesson must occupy consecutive teaching periods with no break between them."
    }
  },
  "additionalProperties": false
}"#;

/// The JSON Schema for one constraint type's parameters.
#[must_use]
pub fn schema_of(constraint_type: ConstraintType) -> &'static str {
    match constraint_type {
        ConstraintType::DurationContiguous => DURATION_CONTIGUOUS,
        ConstraintType::TeacherClash
        | ConstraintType::StudentClash
        | ConstraintType::RoomClash
        | ConstraintType::RoomTypeRequired
        | ConstraintType::EntityUnavailable
        | ConstraintType::NoTeachingInPeriod
        | ConstraintType::ActivityFixed
        | ConstraintType::ActivityScheduled => NO_PARAMETERS,
    }
}

/// Parses a stored `params` string into typed options.
///
/// # Validated against the schema's intent, not by a schema engine
///
/// There is no JSON Schema validator in the dependency tree, and adding one to validate two
/// shapes — an empty object, and an object with one optional boolean — would be a licence
/// review and a dependency for no gain. What the schemas above are *for* is the configuration
/// form; what a write needs is this function, and the two are kept honest by a test that reads
/// each schema's declared `properties` and asserts this parser accepts exactly those.
///
/// When a third parameterised type arrives, that calculus changes and a validator earns its
/// place. Recorded here so the decision is visible rather than inferred from its absence.
///
/// # Errors
///
/// [`ConstraintError::InvalidParams`] when the text is not a JSON object, carries a field the
/// type does not define, or gives a field the wrong type. Never a silent default: a stored
/// value that cannot be read is not a value to guess at, which is `CLAUDE.md`'s rule about
/// persisted data applied to configuration.
pub(crate) fn parse(
    constraint_type: ConstraintType,
    params: &str,
) -> Result<ConstraintOptions, ConstraintError> {
    let invalid = |reason: String| ConstraintError::InvalidParams {
        key: constraint_type.key(),
        reason,
    };

    let value: serde_json::Value = serde_json::from_str(params)
        .map_err(|error| invalid(format!("it is not valid JSON ({error})")))?;
    let object = value
        .as_object()
        .ok_or_else(|| invalid(String::from("the parameters must be a JSON object")))?;

    let mut options = ConstraintOptions::default();
    for (field, held) in object {
        match (constraint_type, field.as_str()) {
            (ConstraintType::DurationContiguous, "allowBreakSpan") => {
                options.allow_break_span = held.as_bool().ok_or_else(|| {
                    invalid(String::from("`allowBreakSpan` must be true or false"))
                })?;
            }
            _ => {
                return Err(invalid(format!(
                    "`{field}` is not a parameter of this rule"
                )));
            }
        }
    }
    Ok(options)
}

#[cfg(test)]
mod tests {
    use evara_core::scheduling::{ConstraintOptions, ConstraintType};

    use super::{parse, schema_of};

    #[test]
    fn every_type_publishes_a_schema_that_parses() {
        // ADR 0005 makes the schema one of the five artefacts a type needs before it is
        // "supported". A schema that is not valid JSON could not generate a form.
        for kind in ConstraintType::all() {
            let schema: serde_json::Value = serde_json::from_str(schema_of(*kind))
                .unwrap_or_else(|error| panic!("{}'s schema does not parse: {error}", kind.key()));
            assert_eq!(schema["type"], "object", "{}", kind.key());
            assert_eq!(
                schema["additionalProperties"],
                false,
                "{} must reject fields it does not define, so a typo is an error rather than \
                 a silently ignored setting",
                kind.key()
            );
        }
    }

    #[test]
    fn the_parser_accepts_exactly_the_fields_the_schema_declares() {
        // The drift ADR 0005 calls "a real defect class", caught by reading the schema rather
        // than by remembering to update two places.
        for kind in ConstraintType::all() {
            let schema: serde_json::Value =
                serde_json::from_str(schema_of(*kind)).expect("the schema parses");
            let declared: Vec<String> = schema["properties"]
                .as_object()
                .expect("properties is an object")
                .keys()
                .cloned()
                .collect();

            for field in &declared {
                // A boolean is the only parameter type the MVP has; when that changes this
                // test needs a value per declared type, and the `match` below will say so.
                let json = format!("{{\"{field}\": true}}");
                assert!(
                    parse(*kind, &json).is_ok(),
                    "{} declares `{field}` and the parser rejected it",
                    kind.key()
                );
            }

            assert!(
                parse(*kind, "{\"notAField\": true}").is_err(),
                "{} accepted a field it does not declare",
                kind.key()
            );
        }
    }

    #[test]
    fn an_empty_object_gives_the_defaults() {
        for kind in ConstraintType::all() {
            assert_eq!(
                parse(*kind, "{}").expect("an empty object is always valid"),
                ConstraintOptions::default()
            );
        }
    }

    #[test]
    fn the_one_mvp_parameter_is_read() {
        let kind = ConstraintType::DurationContiguous;
        assert!(
            !parse(kind, "{}").expect("valid").allow_break_span,
            "the stricter reading is the default — ADR 0033 §5"
        );
        assert!(
            parse(kind, "{\"allowBreakSpan\": true}")
                .expect("valid")
                .allow_break_span
        );
        assert!(
            !parse(kind, "{\"allowBreakSpan\": false}")
                .expect("valid")
                .allow_break_span
        );
    }

    #[test]
    fn malformed_parameters_are_refused_with_a_reason_rather_than_defaulted() {
        let kind = ConstraintType::DurationContiguous;
        for (params, expected) in [
            ("not json at all", "not valid JSON"),
            ("[]", "must be a JSON object"),
            ("\"\"", "must be a JSON object"),
            ("{\"allowBreakSpan\": \"yes\"}", "must be true or false"),
            ("{\"allowBreakSpan\": 1}", "must be true or false"),
            ("{\"allow_break_span\": true}", "is not a parameter"),
        ] {
            let error = parse(kind, params).expect_err(&format!("{params} must be refused"));
            let message = error.to_string();
            assert!(
                message.contains(expected),
                "{params}: expected {expected:?} in {message:?}"
            );
            assert!(
                message.contains("duration-contiguous"),
                "the error must name the rule: {message}"
            );
        }
    }

    #[test]
    fn a_parameterless_rule_refuses_parameters() {
        // Not ignored: a row carrying `allowBreakSpan` for `teacher-clash` is a mistake, and
        // accepting it would leave somebody believing they had configured something.
        let error =
            parse(ConstraintType::TeacherClash, "{\"allowBreakSpan\": true}").expect_err("refused");
        assert!(error.to_string().contains("not a parameter"));
    }
}
