//! The OpenAPI schema of an [`ApiError`].

use http::StatusCode;
use serde_json::Value;
use utoipa::openapi::RefOr;
use utoipa::openapi::schema::{Discriminator, ObjectBuilder, OneOfBuilder, Schema, Type};

use crate::ApiError;

/// One error code an [`ApiError`] can answer, with its status and the schema of its `data`.
pub struct CodeSchema {
    error_code: &'static str,
    status: StatusCode,
    data: RefOr<Schema>,
}

impl std::fmt::Debug for CodeSchema {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CodeSchema")
            .field("error_code", &self.error_code)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

impl CodeSchema {
    /// The code `error_code`, answered with `status` and a `data` of the schema `data`.
    pub fn new(error_code: &'static str, status: StatusCode, data: RefOr<Schema>) -> Self {
        Self {
            error_code,
            status,
            data,
        }
    }

    fn body(&self) -> RefOr<Schema> {
        let code = ObjectBuilder::new()
            .schema_type(Type::String)
            .enum_values(Some([self.error_code]));
        let body = ObjectBuilder::new()
            .property("error_code", code)
            .property("message", ObjectBuilder::new().schema_type(Type::String))
            .property("data", self.data.clone())
            .required("error_code")
            .required("message")
            .required("data");
        RefOr::T(body.into())
    }
}

/// The schema of `E`'s error bodies, one for each of its error codes, told apart by `error_code`.
///
/// A code `E` has twice, such as through a `transparent` variant, appears once. The derive
/// already refuses two variants with one code.
///
/// # Panics
///
/// If `E` has a code twice with another status or another `data`.
pub fn schema<E: ApiError>() -> Schema {
    let mut codes: Vec<CodeSchema> = Vec::new();
    for code in E::code_schemas() {
        match codes.iter().find(|seen| seen.error_code == code.error_code) {
            None => codes.push(code),
            Some(seen) => assert!(
                seen.status == code.status && same(&seen.data, &code.data),
                "the error code {:?} has two statuses or two kinds of data",
                code.error_code
            ),
        }
    }

    let mut one_of = OneOfBuilder::new().discriminator(Some(Discriminator::new("error_code")));
    for code in &codes {
        one_of = one_of.item(code.body());
    }
    Schema::OneOf(one_of.build())
}

/// Whether `a` and `b` are the same schema, whatever the order of their required fields.
fn same(a: &RefOr<Schema>, b: &RefOr<Schema>) -> bool {
    let normal = |schema| serde_json::to_value(schema).ok().map(sort_required);
    normal(a) == normal(b)
}

fn sort_required(mut value: Value) -> Value {
    match &mut value {
        Value::Object(map) => {
            for (key, inner) in map.iter_mut() {
                *inner = sort_required(inner.take());
                if let (true, Value::Array(names)) = (key == "required", inner) {
                    names.sort_by_key(ToString::to_string);
                }
            }
        }
        Value::Array(items) => {
            for item in items {
                *item = sort_required(item.take());
            }
        }
        _ => {}
    }
    value
}

#[cfg(test)]
mod tests {
    use uima_testing::check;

    use super::*;

    #[test]
    fn a_code_schema_debugs_as_its_code_and_status() {
        let code = CodeSchema::new(
            "not-found",
            StatusCode::NOT_FOUND,
            crate::__private::null_schema(),
        );
        let debug = format!("{code:?}");
        check!(debug.contains("not-found") && debug.contains("404"));
    }
}
