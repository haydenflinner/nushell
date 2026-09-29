use nu_engine::command_prelude::*;
use nu_protocol::{EnumDef, EnumValue, TypeDefKind, shell_error::generic::GenericError};

/// Rebuilds a value of a user-declared `enum` type from its base record.
///
/// This command is what `Type.from-record <record>` compiles to; the parser
/// emits calls to it. It is the inverse of how enum values serialize:
/// `{kind: <variant>, ...}` becomes an enum value again, with the variant
/// tag and payload validated against the declaration.
#[derive(Clone)]
pub struct EnumFromRecord;

impl Command for EnumFromRecord {
    fn name(&self) -> &str {
        "enum-from-record"
    }

    fn description(&self) -> &str {
        "Rebuild a declared enum value from its base record (internal: `Type.from-record` compiles to this)."
    }

    fn signature(&self) -> nu_protocol::Signature {
        Signature::build("enum-from-record")
            .input_output_types(vec![(Type::Nothing, Type::Any)])
            .required(
                "type_name",
                SyntaxShape::String,
                "Name of the declared enum type.",
            )
            .required(
                "record",
                SyntaxShape::Any,
                "The base record to rebuild from.",
            )
            .category(Category::Core)
    }

    fn run(
        &self,
        engine_state: &EngineState,
        stack: &mut Stack,
        call: &Call,
        _input: PipelineData,
    ) -> Result<PipelineData, ShellError> {
        let type_name: String = call.req(engine_state, stack, 0)?;
        let record: Value = call.req(engine_state, stack, 1)?;

        // `type_name` may carry an instantiation (`Pair<Option>`) — look up
        // the base name, and substitute the arguments into the declared
        // variant payloads before validating the base record.
        let lookup_name = type_name.split('<').next().unwrap_or(&type_name);
        let Some(type_def) = engine_state.find_type_name(lookup_name.as_bytes(), &[]) else {
            return Err(GenericError::new(
                format!("unknown type `{type_name}`"),
                "no `type` declaration with this name is in scope",
                call.head,
            )
            .into());
        };

        let TypeDefKind::Enum(enum_def) = &type_def.kind else {
            return Err(GenericError::new(
                format!("`{type_name}` is not an enum type"),
                "only enum types can be rebuilt from base records",
                call.head,
            )
            .into());
        };

        let arg_shapes =
            nu_protocol::validate::instantiation_arg_shapes(&type_name).unwrap_or_default();
        let bindings: std::collections::HashMap<&str, &SyntaxShape> = type_def
            .params
            .iter()
            .map(String::as_str)
            .zip(arg_shapes.iter())
            .collect();
        let instantiated = EnumDef {
            variants: enum_def
                .variants
                .iter()
                .map(|v| nu_protocol::EnumVariant {
                    name: v.name.clone(),
                    payload: v.payload.as_ref().map(|p| p.substitute(&bindings)),
                })
                .collect(),
        };

        let enum_name = String::from_utf8_lossy(&type_def.name).to_string();
        EnumValue::from_base_record(record, &enum_name, &instantiated, call.head)
            .map(|v| v.into_pipeline_data())
    }
}
