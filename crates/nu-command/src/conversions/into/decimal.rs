use nu_cmd_base::input_handler::{CellPathOnlyArgs, operate};
use nu_engine::command_prelude::*;
use rust_decimal::Decimal;

#[derive(Clone)]
pub struct IntoDecimal;

impl Command for IntoDecimal {
    fn name(&self) -> &str {
        "into decimal"
    }

    fn signature(&self) -> Signature {
        Signature::build("into decimal")
            .input_output_types(vec![
                (Type::Int, Type::Decimal),
                (Type::String, Type::Decimal),
                (Type::Bool, Type::Decimal),
                (Type::Float, Type::Decimal),
                (Type::Decimal, Type::Decimal),
                (Type::table(), Type::table()),
                (Type::record(), Type::record()),
                (
                    Type::List(Box::new(Type::Any)),
                    Type::List(Box::new(Type::Decimal)),
                ),
            ])
            .rest(
                "rest",
                SyntaxShape::CellPath,
                "For a data structure input, convert data at the given cell paths.",
            )
            .allow_variants_without_examples(true)
            .category(Category::Conversions)
    }

    fn description(&self) -> &str {
        "Convert data into decimal number."
    }

    fn search_terms(&self) -> Vec<&str> {
        vec!["convert", "number", "decimal", "precision"]
    }

    fn run(
        &self,
        engine_state: &EngineState,
        stack: &mut Stack,
        call: &Call,
        input: PipelineData,
    ) -> Result<PipelineData, ShellError> {
        let cell_paths: Vec<CellPath> = call.rest(engine_state, stack, 0)?;
        let args = CellPathOnlyArgs::from(cell_paths);
        operate(action, args, input, call.head, engine_state.signals())
    }

    fn examples(&self) -> Vec<Example<'_>> {
        vec![
            Example {
                description: "Convert string to decimal in table",
                example: "[[num]; ['5.01']] | into decimal num",
                result: Some(Value::test_list(vec![Value::test_record(record! {
                    "num" => Value::decimal(Decimal::new(501, 2), Span::test_data()),
                })])),
            },
            Example {
                description: "Convert string to decimal number",
                example: "'1.345' | into decimal",
                result: Some(Value::decimal(Decimal::new(1345, 3), Span::test_data())),
            },
            Example {
                description: "Coerce list of ints and floats to decimal",
                example: "[4 -5.9] | into decimal",
                result: Some(Value::test_list(vec![
                    Value::decimal(Decimal::new(4, 0), Span::test_data()),
                    Value::decimal(Decimal::new(-59, 1), Span::test_data()),
                ])),
            },
            Example {
                description: "Convert boolean to decimal",
                example: "true | into decimal",
                result: Some(Value::decimal(Decimal::new(1, 0), Span::test_data())),
            },
            Example {
                description: "Convert float to decimal",
                example: "1.23 | into decimal",
                result: Some(Value::decimal(Decimal::new(123, 2), Span::test_data())),
            },
        ]
    }
}

fn action(input: &Value, _args: &CellPathOnlyArgs, head: Span) -> Value {
    let span = input.span();
    match input {
        Value::Decimal { .. } => input.clone(),
        Value::String { val: s, .. } => {
            let other = s.trim();

            match other.parse::<Decimal>() {
                Ok(x) => Value::decimal(x, head),
                Err(_) => Value::error(
                    ShellError::CantConvert {
                        to_type: "decimal".to_string(),
                        from_type: "string".to_string(),
                        span,
                        help: None,
                    },
                    span,
                ),
            }
        }
        Value::Int { val: v, .. } => Value::decimal(Decimal::from(*v), span),
        Value::Float { val: f, .. } => {
            // Convert float to decimal by parsing its string representation
            // This preserves precision better than direct conversion
            match Decimal::try_from(*f) {
                Ok(d) => Value::decimal(d, span),
                Err(_) => {
                    // If direct conversion fails, try parsing the string representation
                    match Decimal::from_str_exact(&format!("{:.15}", f)) {
                        Ok(d) => Value::decimal(d, span),
                        Err(reason) => Value::error(
                            ShellError::CantConvert {
                                to_type: "decimal".to_string(),
                                from_type: format!("float ({})", f),
                                span,
                                help: Some(format!("Float value ({}) cannot be precisely represented as decimal: {}", f, reason)),
                            },
                            span,
                        ),
                    }
                }
            }
        }
        Value::Bool { val: b, .. } => Value::decimal(
            match b {
                true => Decimal::ONE,
                false => Decimal::ZERO,
            },
            span,
        ),
        // Propagate errors by explicitly matching them before the final case.
        Value::Error { .. } => input.clone(),
        other => Value::error(
            ShellError::OnlySupportsThisInputType {
                exp_input_type: "string, int, float, bool or decimal".into(),
                wrong_type: other.get_type().to_string(),
                dst_span: head,
                src_span: other.span(),
            },
            head,
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nu_protocol::Type::Error;
    use rust_decimal::Decimal;

    #[test]
    fn test_examples() -> nu_test_support::Result {
        nu_test_support::test().examples(IntoDecimal {})
    }

    #[test]
    fn string_to_decimal() {
        let word = Value::test_string("3.1415");
        let expected = Value::decimal(Decimal::new(31415, 4), Span::test_data());

        let actual = action(&word, &CellPathOnlyArgs::from(vec![]), Span::test_data());
        assert_eq!(actual, expected);
    }

    #[test]
    fn communicates_parsing_error_given_an_invalid_decimal_string() {
        let invalid_str = Value::test_string("11.6anra");

        let actual = action(
            &invalid_str,
            &CellPathOnlyArgs::from(vec![]),
            Span::test_data(),
        );

        assert_eq!(actual.get_type(), Error);
    }

    #[test]
    fn int_to_decimal() {
        let input_int = Value::test_int(10);
        let expected = Value::decimal(Decimal::new(10, 0), Span::test_data());
        let actual = action(
            &input_int,
            &CellPathOnlyArgs::from(vec![]),
            Span::test_data(),
        );

        assert_eq!(actual, expected);
    }

    #[test]
    fn float_to_decimal() {
        let input_float = Value::test_float(1.23);
        let expected = Value::decimal(Decimal::new(123, 2), Span::test_data());
        let actual = action(
            &input_float,
            &CellPathOnlyArgs::from(vec![]),
            Span::test_data(),
        );

        assert_eq!(actual, expected);
    }

    #[test]
    fn bool_to_decimal() {
        let input_bool = Value::test_bool(true);
        let expected = Value::decimal(Decimal::ONE, Span::test_data());
        let actual = action(
            &input_bool,
            &CellPathOnlyArgs::from(vec![]),
            Span::test_data(),
        );

        assert_eq!(actual, expected);
    }
}
