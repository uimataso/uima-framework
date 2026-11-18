//! What `#[derive(ApiError)]` generates code with.

use utoipa::openapi::RefOr;
use utoipa::openapi::schema::{ObjectBuilder, Schema, SchemaType, Type};
pub use {http, serde_json, utoipa};

/// The schema of a `null` `data`.
pub fn null_schema() -> RefOr<Schema> {
    RefOr::T(ObjectBuilder::new().schema_type(Type::Null).into())
}

/// The schema of a `debug_data` variant's `data`.
pub fn debug_schema() -> RefOr<Schema> {
    RefOr::T(
        ObjectBuilder::new()
            .schema_type(SchemaType::from_iter([Type::String, Type::Null]))
            .description(Some("The error in a debug build, `null` otherwise."))
            .into(),
    )
}
