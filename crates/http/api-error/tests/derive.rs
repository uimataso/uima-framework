//! What `#[derive(ApiError)]` gives an app's error enum.

use http::StatusCode;
use serde::Serialize;
use serde_json::{Value, json};
use uima_http_api_error::{ApiError, ErrorResponse, FrameworkError};
use uima_testing::check;
use utoipa::{PartialSchema as _, ToSchema};

#[derive(Debug, Serialize, ToSchema)]
struct InvalidName {
    reason: String,
}

#[derive(Debug, thiserror::Error, ApiError)]
enum AppError {
    #[error("Not found")]
    #[api(status = 404)]
    NotFound,

    #[error("Payload too large")]
    #[api(status = 413)]
    PayloadTooLarge { max_size: usize, actual: usize },

    #[error("invalid name")]
    InvalidName(InvalidName),

    #[error("Invalid auth token: {0}")]
    #[api(status = 401, no_data)]
    InvalidAuthToken(String),

    #[error("the photo isn't ready")]
    #[api(status = 409, no_data)]
    PhotoNotReady { photo: u32 },

    #[error(transparent)]
    #[api(transparent)]
    Framework(#[from] FrameworkError),
}

fn response(error: &impl ApiError) -> Value {
    serde_json::to_value(ErrorResponse::new(error)).unwrap()
}

#[test]
fn a_variant_gives_its_status_code_message_and_data() {
    let not_found = AppError::NotFound;
    check!(not_found.status() == StatusCode::NOT_FOUND);
    check!(
        response(&not_found)
            == json!({"error_code": "not-found", "message": "Not found", "data": null})
    );

    let too_large = AppError::PayloadTooLarge {
        max_size: 500,
        actual: 600,
    };
    check!(too_large.status() == StatusCode::PAYLOAD_TOO_LARGE);
    check!(
        response(&too_large)
            == json!({
                "error_code": "payload-too-large",
                "message": "Payload too large",
                "data": {"max_size": 500, "actual": 600},
            })
    );
}

#[test]
fn a_variant_without_a_status_is_a_400() {
    let invalid = AppError::InvalidName(InvalidName {
        reason: "too long".into(),
    });
    check!(invalid.status() == StatusCode::BAD_REQUEST);
}

#[test]
fn a_single_field_is_the_data() {
    let invalid = AppError::InvalidName(InvalidName {
        reason: "too long".into(),
    });
    check!(
        response(&invalid)
            == json!({
                "error_code": "invalid-name",
                "message": "invalid name",
                "data": {"reason": "too long"},
            })
    );
}

#[test]
fn no_data_keeps_the_fields_out_of_the_response() {
    let token = AppError::InvalidAuthToken("expired".into());
    check!(token.status() == StatusCode::UNAUTHORIZED);
    check!(
        response(&token)
            == json!({
                "error_code": "invalid-auth-token",
                "message": "Invalid auth token: expired",
                "data": null,
            })
    );
    check!(response(&AppError::PhotoNotReady { photo: 7 })["data"] == Value::Null);
}

#[test]
fn transparent_gives_the_inner_errors_answer() {
    check!(AppError::from(FrameworkError::NotFound).status() == StatusCode::NOT_FOUND);
    let error = AppError::from(FrameworkError::TooManyItems {
        limit: 100,
        actual: 101,
    });
    check!(error.status() == StatusCode::BAD_REQUEST);
    check!(
        response(&error)
            == json!({
                "error_code": "too-many-items",
                "message": "101 items, over the limit of 100",
                "data": {"limit": 100, "actual": 101},
            })
    );
}

#[test]
fn the_schema_is_one_of_the_error_codes() {
    let schema = serde_json::to_value(AppError::schema()).unwrap();
    check!(schema["discriminator"] == json!({"propertyName": "error_code"}));

    let variants = schema["oneOf"].as_array().unwrap();
    let code = |v: &Value| {
        v["properties"]["error_code"]["enum"][0]
            .as_str()
            .unwrap()
            .to_owned()
    };
    let find = |c: &str| variants.iter().find(|v| code(v) == c).unwrap().clone();

    // The app's codes and the framework's, `not-found` once though both have it.
    let codes: Vec<String> = variants.iter().map(code).collect();
    for c in [
        "not-found",
        "payload-too-large",
        "invalid-name",
        "invalid-auth-token",
        "photo-not-ready",
        "invalid-json",
        "too-many-items",
        "internal-error",
    ] {
        check!(codes.iter().filter(|o| *o == c).count() == 1, "{c}");
    }

    for v in variants {
        check!(v["required"] == json!(["error_code", "message", "data"]));
        check!(v["properties"]["error_code"]["type"] == "string");
        check!(v["properties"]["message"] == json!({"type": "string"}));
    }
    check!(find("not-found")["properties"]["data"] == json!({"type": "null"}));
    check!(find("invalid-auth-token")["properties"]["data"] == json!({"type": "null"}));
    let too_large = find("payload-too-large")["properties"]["data"].clone();
    check!(too_large["type"] == "object");
    check!(too_large["required"] == json!(["max_size", "actual"]));
    check!(too_large["properties"]["max_size"] == serde_json::to_value(usize::schema()).unwrap());
    check!(find("internal-error")["properties"]["data"]["type"] == json!(["string", "null"]));
    check!(
        find("invalid-name")["properties"]["data"]
            == serde_json::to_value(InvalidName::schema()).unwrap()
    );
    check!(find("too-many-items")["properties"]["data"]["required"] == json!(["limit", "actual"]));
}

#[test]
fn the_schema_is_named_for_the_enum() {
    check!(<AppError as ToSchema>::name() == "AppError");
}

mod clash {
    use super::*;

    #[derive(Debug, thiserror::Error, ApiError)]
    #[expect(dead_code, reason = "only its schema is used")]
    enum OtherData {
        #[error("too many")]
        TooManyItems { limit: String },

        #[error(transparent)]
        #[api(transparent)]
        Framework(FrameworkError),
    }

    #[derive(Debug, thiserror::Error, ApiError)]
    #[expect(dead_code, reason = "only its schema is used")]
    enum OtherStatus {
        #[error("not found")]
        #[api(status = 410)]
        NotFound,

        #[error(transparent)]
        #[api(transparent)]
        Framework(FrameworkError),
    }

    #[test]
    fn a_code_with_two_answers_panics_in_the_schema() {
        let data = std::panic::catch_unwind(OtherData::schema).err().unwrap();
        check!(
            data.downcast_ref::<String>()
                .unwrap()
                .contains("too-many-items")
        );
        let status = std::panic::catch_unwind(OtherStatus::schema).err().unwrap();
        check!(
            status
                .downcast_ref::<String>()
                .unwrap()
                .contains("not-found")
        );
    }
}

mod transparent {
    use super::*;

    #[derive(Debug, thiserror::Error, ApiError)]
    enum Inner {
        #[error("gone")]
        #[api(status = 410)]
        Gone,
    }

    // Inherent methods named like the trait's don't answer for a transparent variant.
    impl Inner {
        fn status(&self) -> u16 {
            418
        }

        fn error_code(&self) -> &'static str {
            "inherent"
        }

        fn data(&self) -> &'static str {
            "inherent"
        }
    }

    #[derive(Debug, thiserror::Error, ApiError)]
    enum Outer {
        #[error(transparent)]
        #[api(transparent)]
        Inner(Inner),
    }

    #[test]
    fn the_inner_error_answers_through_the_trait() {
        let inner = Inner::Gone;
        check!((inner.status(), inner.error_code(), inner.data()) == (418, "inherent", "inherent"));
        let outer = Outer::Inner(Inner::Gone);
        check!(ApiError::status(&outer) == StatusCode::GONE);
        check!(response(&outer) == json!({"error_code": "gone", "message": "gone", "data": null}));
    }
}

mod names {
    use super::*;

    #[derive(Debug, thiserror::Error, ApiError)]
    #[expect(dead_code, reason = "only its name is used")]
    enum r#Enum {
        #[error("e")]
        E,
    }

    #[derive(Debug, thiserror::Error, ApiError)]
    enum Names {
        #[error("raw")]
        r#Type { r#type: u32, map: u32 },
    }

    #[test]
    fn a_raw_name_is_answered_without_its_prefix() {
        let error = Names::r#Type { r#type: 1, map: 2 };
        check!(response(&error)["error_code"] == "type");
        check!(response(&error)["data"] == json!({"type": 1, "map": 2}));
        let schema = serde_json::to_value(Names::schema()).unwrap();
        check!(schema["oneOf"][0]["properties"]["data"]["required"] == json!(["type", "map"]));
        check!(<r#Enum as ToSchema>::name() == "Enum");
    }
}

mod components {
    use super::*;

    #[derive(Debug, Serialize, ToSchema)]
    struct Bar {
        n: u32,
    }

    #[derive(Debug, Serialize, ToSchema)]
    struct Foo {
        bar: Bar,
    }

    #[derive(Debug, Serialize, ToSchema)]
    struct Qux {
        n: u32,
    }

    #[derive(Debug, Serialize, ToSchema)]
    struct Corge {
        qux: Qux,
    }

    #[derive(Debug, Serialize, ToSchema)]
    struct Quux {
        n: u32,
    }

    #[derive(Debug, Serialize, ToSchema)]
    struct Baz {
        quux: Quux,
    }

    #[derive(Debug, thiserror::Error, ApiError)]
    #[expect(dead_code, reason = "only its schema is used")]
    enum Inner {
        #[error("baz")]
        Baz(Baz),
    }

    #[derive(Debug, thiserror::Error, ApiError)]
    #[expect(dead_code, reason = "only its schema is used")]
    enum Nested {
        #[error("foo")]
        Foo(Foo),
        #[error("corge")]
        Corge { corge: Corge },
        #[error(transparent)]
        #[api(transparent)]
        Inner(Inner),
    }

    #[test]
    fn the_schemas_the_data_refers_to_are_listed() {
        let mut schemas = Vec::new();
        <Nested as ToSchema>::schemas(&mut schemas);
        let names: Vec<&str> = schemas.iter().map(|(name, _)| name.as_str()).collect();
        // Through a single field, named fields and a transparent variant.
        for name in ["Bar", "Qux", "Quux"] {
            check!(names.contains(&name), "{name}");
        }
        let data =
            serde_json::to_value(Nested::schema()).unwrap()["oneOf"][0]["properties"]["data"]
                .clone();
        check!(data["properties"]["bar"] == json!({"$ref": "#/components/schemas/Bar"}));
    }
}

mod code {
    use super::*;

    #[derive(Debug, thiserror::Error, ApiError)]
    enum Codes {
        #[error("oauth")]
        #[api(code = "oauth-failed", status = 401)]
        OAuthFailed,
        #[error("oauth2")]
        #[api(code = "oauth2-failed", status = 599)]
        OAuth2Failed,
    }

    #[derive(Debug, thiserror::Error, ApiError)]
    enum Defaults {
        #[error("oauth")]
        OAuthFailed,
    }

    #[test]
    fn code_names_the_error_code() {
        check!(Defaults::OAuthFailed.error_code() == "o-auth-failed");
        check!(Codes::OAuthFailed.error_code() == "oauth-failed");
        check!(Codes::OAuth2Failed.error_code() == "oauth2-failed");
        check!(Codes::OAuth2Failed.status().as_u16() == 599);
        let schema = serde_json::to_value(Codes::schema()).unwrap();
        check!(schema["oneOf"][0]["properties"]["error_code"]["enum"] == json!(["oauth-failed"]));
    }
}

mod unserializable {
    use std::borrow::Cow;

    use serde::Serializer;
    use utoipa::PartialSchema;
    use utoipa::openapi::{RefOr, Schema};

    use super::*;

    #[derive(Debug)]
    struct Unserializable;

    impl Serialize for Unserializable {
        fn serialize<S: Serializer>(&self, _: S) -> Result<S::Ok, S::Error> {
            Err(serde::ser::Error::custom("can't"))
        }
    }

    impl PartialSchema for Unserializable {
        fn schema() -> RefOr<Schema> {
            String::schema()
        }
    }

    impl ToSchema for Unserializable {
        fn name() -> Cow<'static, str> {
            Cow::Borrowed("Unserializable")
        }
    }

    #[derive(Debug, thiserror::Error, ApiError)]
    enum Broken {
        #[error("broken")]
        Field(Unserializable),
        #[error("broken")]
        Fields { field: Unserializable },
    }

    #[test]
    fn data_that_doesnt_serialize_panics_in_a_debug_build() {
        for error in [
            Broken::Field(Unserializable),
            Broken::Fields {
                field: Unserializable,
            },
        ] {
            let data = std::panic::catch_unwind(|| error.data());
            if cfg!(debug_assertions) {
                let panic = data.err().unwrap();
                let message = panic.downcast_ref::<String>().unwrap();
                check!(
                    message.contains("can't") && message.contains("`field"),
                    "{message}"
                );
            } else {
                let data = data.unwrap().map(|d| d.to_string());
                check!(matches!(
                    data.as_deref(),
                    Some("null" | r#"{"field":null}"#)
                ));
            }
        }
    }
}

mod from_a_macro {
    use super::*;

    // The inner type comes from another hygiene context than the enum.
    macro_rules! wrap {
        ($inner:tt) => {
            #[derive(Debug, thiserror::Error, ApiError)]
            enum Wrapped {
                #[error(transparent)]
                #[api(transparent)]
                Inner($inner),
            }
        };
    }

    wrap!(FrameworkError);

    #[test]
    fn a_transparent_type_from_a_macro_works() {
        check!(Wrapped::Inner(FrameworkError::NotFound).status() == StatusCode::NOT_FOUND);
        check!(!Wrapped::code_schemas().is_empty());
    }
}

mod shadowing {
    #![allow(non_upper_case_globals, reason = "to shadow the generated locals")]

    use super::*;

    const codes: u8 = 0;
    const field: u8 = 0;
    const inner: u8 = 0;
    const value: u8 = 0;
    const e: u8 = 0;
    const n: u32 = 0;

    // `Display` by hand, since thiserror's derive binds `{ n }` too.
    #[derive(Debug, ApiError)]
    enum Shadowed {
        One(u32),
        Named {
            n: u32,
        },
        #[api(status = 500, debug_data)]
        Debug(String),
        #[api(transparent)]
        Framework(FrameworkError),
    }

    impl std::fmt::Display for Shadowed {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("shadowed")
        }
    }

    #[test]
    fn the_apps_constants_dont_shadow_the_generated_code() {
        check!(Shadowed::One(1).data() == Some(json!(1)));
        check!(Shadowed::Named { n: 1 }.data() == Some(json!({"n": 1})));
        let debug = Shadowed::Debug("why".into()).data();
        check!(debug == cfg!(debug_assertions).then(|| json!(r#""why""#)));
        check!(Shadowed::Framework(FrameworkError::NotFound).status() == StatusCode::NOT_FOUND);
        check!(Shadowed::code_schemas().len() > 3);
        check!([codes, field, inner, value, e] == [0; 5] && n == 0);
    }
}

mod order {
    use super::*;

    #[derive(Debug, thiserror::Error, ApiError)]
    #[expect(dead_code, reason = "only its schema is used")]
    enum Reordered {
        #[error("too many")]
        TooManyItems { actual: usize, limit: usize },
        #[error(transparent)]
        #[api(transparent)]
        Framework(FrameworkError),
    }

    #[test]
    fn the_same_data_in_another_field_order_is_the_same_code() {
        let schema = serde_json::to_value(Reordered::schema()).unwrap();
        let count = schema["oneOf"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|v| v["properties"]["error_code"]["enum"][0] == "too-many-items")
            .count();
        check!(count == 1);
    }
}
