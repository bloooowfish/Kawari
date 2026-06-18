use axum::response::{Html, Redirect};
use axum::routing::post;
use axum::{
    Router,
    extract::{Form, Query},
    routing::get,
};
use kawari::common::{BasicCharacterData, User};
use kawari::config::get_config;
use kawari::ipc::kawari::{
    CustomIpcData, CustomIpcSegment, HOUSING_ADMIN_GREETING_MAX_BYTES,
    HOUSING_ADMIN_NAME_MAX_BYTES, clamp_housing_admin_greeting_for_ipc,
    clamp_housing_admin_name_for_ipc, validate_housing_import_path_for_ipc,
};
use kawari::packet::send_custom_world_packet;
use kawari::web_static_dir;
use minijinja::context;
use minijinja::{Environment, path_loader};
use serde::Deserialize;
use std::fmt::Write as _;
use tower_http::services::ServeDir;

fn setup_default_environment() -> Environment<'static> {
    let mut env = Environment::new();
    env.set_loader(path_loader("resources/web/templates"));

    env
}

async fn root() -> Html<String> {
    let config = get_config();

    let environment = setup_default_environment();
    let template = environment.get_template("admin_general.html").unwrap();
    Html(template.render(context! { config }).unwrap())
}

async fn users() -> Html<String> {
    let environment = setup_default_environment();
    let template = environment.get_template("admin_users.html").unwrap();
    let config = get_config();

    let Ok(mut login_reply) =
        ureq::get(&*format!("{}/_private/users", config.login.server_name)).call()
    else {
        // TODO: add a better error message here
        tracing::warn!("Failed to contact login server, is it running?");
        return Html(template.render(context! {}).unwrap());
    };

    let Ok(body) = login_reply.body_mut().read_to_string() else {
        // TODO: add a better error message here
        tracing::warn!("Failed to contact login server, is it running?");
        return Html(template.render(context! {}).unwrap());
    };

    let users: Option<Vec<User>> = serde_json::from_str(&body).ok();

    Html(template.render(context! { users }).unwrap())
}

async fn characters() -> Html<String> {
    let environment = setup_default_environment();
    let template = environment.get_template("admin_characters.html").unwrap();

    let ipc_segment = CustomIpcSegment::new(CustomIpcData::RequestFullCharacterList {});

    if let Some(response) = send_custom_world_packet(ipc_segment).await
        && let CustomIpcData::FullCharacterListResponse { json } = response.data
    {
        let characters: Option<Vec<BasicCharacterData>> = serde_json::from_str(&json).ok();
        Html(template.render(context! { characters }).unwrap())
    } else {
        // error out better than this
        Html(template.render(context! {}).unwrap())
    }
}

#[derive(Deserialize, Default)]
struct HousingQuery {
    land_ident: Option<i64>,
    status: Option<String>,
    message: Option<String>,
}

#[derive(Deserialize)]
struct HousingLandIdentForm {
    land_ident: i64,
}

#[derive(Deserialize)]
struct HousingResetEstateForm {
    land_ident: i64,
    confirm_reset: Option<String>,
}

#[derive(Deserialize)]
struct HousingUpdateTextForm {
    land_ident: i64,
    name: String,
    greeting: String,
}

#[derive(Deserialize)]
struct HousingImportForm {
    path: String,
}

fn build_update_housing_estate_text_request(input: &HousingUpdateTextForm) -> CustomIpcData {
    CustomIpcData::UpdateHousingEstateText {
        land_ident: input.land_ident,
        name: clamp_housing_admin_name_for_ipc(&input.name),
        greeting: clamp_housing_admin_greeting_for_ipc(&input.greeting),
    }
}

fn update_housing_estate_text_warning(input: &HousingUpdateTextForm) -> Option<String> {
    let mut warnings = Vec::new();

    if clamp_housing_admin_name_for_ipc(&input.name) != input.name {
        warnings.push(format!(
            "Estate name was clamped to the {}-byte housing payload limit.",
            HOUSING_ADMIN_NAME_MAX_BYTES
        ));
    }

    if clamp_housing_admin_greeting_for_ipc(&input.greeting) != input.greeting {
        warnings.push(format!(
            "Greeting was clamped to the {}-byte housing payload limit.",
            HOUSING_ADMIN_GREETING_MAX_BYTES
        ));
    }

    if warnings.is_empty() {
        None
    } else {
        Some(warnings.join(" "))
    }
}

fn build_import_housing_estate_request(path: &str) -> Result<CustomIpcData, String> {
    validate_housing_import_path_for_ipc(path)
        .map(|path| CustomIpcData::ImportHousingEstate { path })
}

fn housing_query_value(value: Option<&str>) -> Option<&str> {
    value.map(str::trim).filter(|value| !value.is_empty())
}

fn housing_status_label(status: &str) -> String {
    match status.trim().to_ascii_lowercase().as_str() {
        "success" => "Success".to_string(),
        "warning" => "Warning".to_string(),
        "error" => "Error".to_string(),
        _ => status.trim().to_string(),
    }
}

fn housing_query_status_message(query: &HousingQuery) -> Option<String> {
    let status = housing_query_value(query.status.as_deref());
    let message = housing_query_value(query.message.as_deref());

    match (status, message) {
        (Some(status), Some(message)) => {
            Some(format!("{}: {message}", housing_status_label(status)))
        }
        (Some(status), None) => Some(housing_status_label(status)),
        (None, Some(message)) => Some(message.to_string()),
        (None, None) => None,
    }
}

fn percent_encode_query_value(value: &str) -> String {
    let mut encoded = String::new();

    for byte in value.as_bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                encoded.push(*byte as char)
            }
            _ => write!(&mut encoded, "%{byte:02X}").expect("writing to a String cannot fail"),
        }
    }

    encoded
}

fn housing_redirect_location(
    selected_land_ident: Option<i64>,
    status: &str,
    message: &str,
) -> String {
    let mut params = Vec::new();

    if let Some(land_ident) = selected_land_ident {
        params.push(format!("land_ident={land_ident}"));
    }

    if let Some(status) = housing_query_value(Some(status)) {
        params.push(format!("status={}", percent_encode_query_value(status)));
    }

    if let Some(message) = housing_query_value(Some(message)) {
        params.push(format!("message={}", percent_encode_query_value(message)));
    }

    if params.is_empty() {
        "/housing".to_string()
    } else {
        format!("/housing?{}", params.join("&"))
    }
}

fn housing_redirect_after_post(
    selected_land_ident: Option<i64>,
    status: &str,
    message: &str,
) -> Redirect {
    Redirect::to(&housing_redirect_location(
        selected_land_ident,
        status,
        message,
    ))
}

fn housing_status_for_message(message: &str) -> &'static str {
    let message = message.trim();
    if message.starts_with("Failed")
        || message.starts_with("Unexpected")
        || message.starts_with("World server did not respond")
        || message.contains(" was not found")
        || message.contains("Import path")
        || message.contains("Confirmation checkbox")
    {
        "error"
    } else {
        "success"
    }
}

fn housing_mutation_response_message(
    response: Option<CustomIpcSegment>,
    unexpected_message: &str,
    timeout_message: &str,
) -> String {
    match response {
        Some(response) => match response.data {
            CustomIpcData::HousingEstateMutationResult { message } => message,
            _ => unexpected_message.to_string(),
        },
        None => timeout_message.to_string(),
    }
}

fn housing_export_response_message(
    response: Option<CustomIpcSegment>,
    unexpected_message: &str,
    timeout_message: &str,
) -> String {
    match response {
        Some(response) => match response.data {
            CustomIpcData::HousingEstateExported { path, message } => {
                if path.is_empty() {
                    message
                } else {
                    format!("{message} Path: {path}")
                }
            }
            _ => unexpected_message.to_string(),
        },
        None => timeout_message.to_string(),
    }
}

#[derive(Debug, Default, PartialEq)]
struct HousingSummaryView {
    estates: Vec<serde_json::Value>,
    status_message: Option<String>,
}

#[derive(Debug, Default, PartialEq)]
struct HousingDetailView {
    selected_estate: Option<serde_json::Value>,
    pretty_json: String,
}

fn parse_housing_summary_error_message(error: &str, truncated: bool) -> String {
    match error {
        "housing_summary_ipc_overflow" if truncated => {
            "Housing summary exceeded the admin IPC payload limit; no estate rows were loaded."
                .to_string()
        }
        other if truncated => {
            format!("Housing summary request failed with truncated error payload: {other}")
        }
        other => format!("Housing summary request failed: {other}"),
    }
}

fn parse_housing_summary_json(json: &str) -> HousingSummaryView {
    match serde_json::from_str::<serde_json::Value>(json) {
        Ok(serde_json::Value::Array(estates)) => HousingSummaryView {
            estates,
            status_message: None,
        },
        Ok(serde_json::Value::Object(object)) => {
            let error = object.get("error").and_then(serde_json::Value::as_str);
            let truncated = object
                .get("truncated")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false);

            if let Some(estates) = object
                .get("estates")
                .or_else(|| object.get("rows"))
                .and_then(serde_json::Value::as_array)
            {
                let status_message = object
                    .get("message")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
                    .or_else(|| {
                        truncated.then(|| {
                            "Housing summary was truncated to fit the admin IPC payload limit."
                                .to_string()
                        })
                    });

                return HousingSummaryView {
                    estates: estates.clone(),
                    status_message,
                };
            }

            let status_message = error
                .map(|value| parse_housing_summary_error_message(value, truncated))
                .or_else(|| {
                    Some("Housing summary returned an unexpected JSON object.".to_string())
                });

            HousingSummaryView {
                estates: Vec::new(),
                status_message,
            }
        }
        Ok(_) => HousingSummaryView {
            estates: Vec::new(),
            status_message: Some("Housing summary returned an unexpected JSON value.".to_string()),
        },
        Err(error) => HousingSummaryView {
            estates: Vec::new(),
            status_message: Some(format!("Failed to parse housing summary JSON: {error}")),
        },
    }
}

fn merge_status_messages(primary: Option<String>, secondary: Option<String>) -> Option<String> {
    match (primary, secondary) {
        (Some(primary), Some(secondary)) => Some(format!("{primary} {secondary}")),
        (Some(primary), None) => Some(primary),
        (None, Some(secondary)) => Some(secondary),
        (None, None) => None,
    }
}

async fn request_housing_summary() -> HousingSummaryView {
    let ipc_segment = CustomIpcSegment::new(CustomIpcData::RequestHousingSummary {});

    if let Some(response) = send_custom_world_packet(ipc_segment).await
        && let CustomIpcData::HousingSummaryResponse { json } = response.data
    {
        parse_housing_summary_json(&json)
    } else {
        HousingSummaryView {
            estates: Vec::new(),
            status_message: Some(
                "World server did not respond to housing summary request.".to_string(),
            ),
        }
    }
}

fn parse_housing_detail_response(response: Option<CustomIpcSegment>) -> HousingDetailView {
    let Some(response) = response else {
        return HousingDetailView {
            pretty_json: "World server did not respond to housing detail request.".to_string(),
            ..Default::default()
        };
    };

    let CustomIpcData::HousingEstateDetailResponse { json } = response.data else {
        return HousingDetailView {
            pretty_json: "Unexpected response while requesting housing detail.".to_string(),
            ..Default::default()
        };
    };

    match serde_json::from_str::<serde_json::Value>(&json) {
        Ok(value) => HousingDetailView {
            selected_estate: value.get("estate").cloned(),
            pretty_json: serde_json::to_string_pretty(&value).unwrap_or(json),
        },
        Err(_) => HousingDetailView {
            pretty_json: json,
            ..Default::default()
        },
    }
}

async fn request_housing_detail(land_ident: i64) -> HousingDetailView {
    let ipc_segment =
        CustomIpcSegment::new(CustomIpcData::RequestHousingEstateDetail { land_ident });

    parse_housing_detail_response(send_custom_world_packet(ipc_segment).await)
}

async fn render_housing_page(
    selected_land_ident: Option<i64>,
    status_message: Option<String>,
) -> Html<String> {
    let environment = setup_default_environment();
    let template = environment.get_template("admin_housing.html").unwrap();
    let summary = request_housing_summary().await;
    let selected_detail = if let Some(land_ident) = selected_land_ident {
        Some(request_housing_detail(land_ident).await)
    } else {
        None
    };
    let status_message = merge_status_messages(status_message, summary.status_message);

    Html(
        template
            .render(context! {
                estates => summary.estates,
                selected_land_ident,
                selected_estate => selected_detail.as_ref().and_then(|detail| detail.selected_estate.clone()),
                selected_detail_json => selected_detail.map(|detail| detail.pretty_json),
                status_message,
                name_max_bytes => HOUSING_ADMIN_NAME_MAX_BYTES,
                greeting_max_bytes => HOUSING_ADMIN_GREETING_MAX_BYTES,
            })
            .unwrap(),
    )
}

async fn housing(Query(query): Query<HousingQuery>) -> Html<String> {
    render_housing_page(query.land_ident, housing_query_status_message(&query)).await
}

async fn reset_housing_furniture(Form(input): Form<HousingLandIdentForm>) -> Redirect {
    let message = housing_mutation_response_message(
        send_custom_world_packet(CustomIpcSegment::new(
            CustomIpcData::ResetHousingFurniture {
                land_ident: input.land_ident,
            },
        ))
        .await,
        "Unexpected response while resetting furniture.",
        "World server did not respond to reset furniture.",
    );

    housing_redirect_after_post(
        Some(input.land_ident),
        housing_status_for_message(&message),
        &message,
    )
}

async fn reset_housing_estate(Form(input): Form<HousingResetEstateForm>) -> Redirect {
    if input.confirm_reset.as_deref() != Some("on") {
        return housing_redirect_after_post(
            Some(input.land_ident),
            "error",
            "Confirmation checkbox is required before resetting an estate.",
        );
    }

    let message = housing_mutation_response_message(
        send_custom_world_packet(CustomIpcSegment::new(CustomIpcData::ResetHousingEstate {
            land_ident: input.land_ident,
        }))
        .await,
        "Unexpected response while resetting estate.",
        "World server did not respond to reset estate.",
    );

    housing_redirect_after_post(None, housing_status_for_message(&message), &message)
}

async fn update_housing_text(Form(input): Form<HousingUpdateTextForm>) -> Redirect {
    let warning = update_housing_estate_text_warning(&input);
    let message = housing_mutation_response_message(
        send_custom_world_packet(CustomIpcSegment::new(
            build_update_housing_estate_text_request(&input),
        ))
        .await,
        "Unexpected response while updating estate text.",
        "World server did not respond to update estate text.",
    );
    let status = if housing_status_for_message(&message) == "error" {
        "error"
    } else if warning.is_some() {
        "warning"
    } else {
        "success"
    };
    let message = merge_status_messages(warning, Some(message)).unwrap_or_default();

    housing_redirect_after_post(Some(input.land_ident), status, &message)
}

async fn export_housing_estate(Form(input): Form<HousingLandIdentForm>) -> Redirect {
    let message = housing_export_response_message(
        send_custom_world_packet(CustomIpcSegment::new(CustomIpcData::ExportHousingEstate {
            land_ident: input.land_ident,
        }))
        .await,
        "Unexpected response while exporting estate.",
        "World server did not respond to export estate.",
    );

    housing_redirect_after_post(
        Some(input.land_ident),
        housing_status_for_message(&message),
        &message,
    )
}

async fn import_housing_estate(Form(input): Form<HousingImportForm>) -> Redirect {
    if input.path.trim().is_empty() {
        return housing_redirect_after_post(None, "error", "Import path is required.");
    }

    let request = match build_import_housing_estate_request(&input.path) {
        Ok(request) => request,
        Err(message) => return housing_redirect_after_post(None, "error", &message),
    };

    let message = housing_mutation_response_message(
        send_custom_world_packet(CustomIpcSegment::new(request)).await,
        "Unexpected response while importing estate.",
        "World server did not respond to import estate.",
    );

    housing_redirect_after_post(None, housing_status_for_message(&message), &message)
}

#[derive(Deserialize, Debug)]
#[allow(dead_code)]
struct Input {
    worlds_open: Option<String>,
    login_open: Option<String>,
    festival0: Option<u16>,
    festival1: Option<u16>,
    festival2: Option<u16>,
    festival3: Option<u16>,
    world: Option<u16>,
    login_message: Option<String>,
}

async fn apply(Form(input): Form<Input>) -> Redirect {
    let mut config = get_config();

    if let Some(gate_open) = input.worlds_open {
        config.frontier.worlds_open = gate_open == "on";
    } else {
        config.frontier.worlds_open = false;
    }

    if let Some(gate_open) = input.login_open {
        config.frontier.login_open = gate_open == "on";
    } else {
        config.frontier.login_open = false;
    }

    config.world.active_festivals = [
        input.festival0.unwrap_or(0),
        input.festival1.unwrap_or(1),
        input.festival2.unwrap_or(2),
        input.festival3.unwrap_or(3),
        // TODO: expose these in the UI
        0,
        0,
        0,
        0,
    ];

    if let Some(world) = input.world {
        config.world.world_id = world;
    }

    if let Some(login_message) = input.login_message {
        config.world.login_message = login_message;
    }

    serde_yaml_ng::to_writer(&std::fs::File::create("config.yaml").unwrap(), &config)
        .expect("TODO: panic message");

    Redirect::to("/")
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();

    let app = Router::new()
        .route("/", get(root))
        .route("/apply", post(apply))
        .route("/users", get(users))
        .route("/characters", get(characters))
        .route("/housing", get(housing))
        .route("/housing/reset_furniture", post(reset_housing_furniture))
        .route("/housing/reset_estate", post(reset_housing_estate))
        .route("/housing/update_text", post(update_housing_text))
        .route("/housing/export", post(export_housing_estate))
        .route("/housing/import", post(import_housing_estate))
        .nest_service("/static", ServeDir::new(web_static_dir!("")));

    let config = get_config();

    let addr = config.admin.get_socketaddr();
    tracing::info!("Server started on {addr}");
    let listener = tokio::net::TcpListener::bind(addr).await.unwrap();
    axum::serve(listener, app).await.unwrap();
}

#[cfg(test)]
mod tests {
    use axum::{
        http::{StatusCode, header},
        response::IntoResponse,
    };
    use kawari::ipc::kawari::{CustomIpcData, CustomIpcSegment};
    use minijinja::{Environment, context, path_loader};
    use std::{fs, path::PathBuf, sync::Mutex};

    use super::{
        HousingQuery, HousingUpdateTextForm, build_import_housing_estate_request,
        housing_export_response_message, housing_query_status_message, housing_redirect_after_post,
        housing_redirect_location, parse_housing_detail_response, parse_housing_summary_json,
        setup_default_environment, update_housing_estate_text_warning,
    };

    static CWD_LOCK: Mutex<()> = Mutex::new(());

    struct CurrentDirGuard {
        original: PathBuf,
    }

    impl CurrentDirGuard {
        fn change_to(path: &std::path::Path) -> Self {
            let original = std::env::current_dir().expect("current directory should be available");
            std::env::set_current_dir(path).expect("test should be able to change current dir");
            Self { original }
        }
    }

    impl Drop for CurrentDirGuard {
        fn drop(&mut self) {
            std::env::set_current_dir(&self.original)
                .expect("test should be able to restore current dir");
        }
    }

    fn test_template_environment() -> Environment<'static> {
        let mut env = Environment::new();
        let template_dir =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../resources/web/templates");
        env.set_loader(path_loader(template_dir));

        env
    }

    #[test]
    fn housing_summary_array_returns_estates_without_status_message() {
        let summary = parse_housing_summary_json(
            r#"[{"land_ident":101,"owner_name":"Tester","furniture_counts":{"total":4}}]"#,
        );

        assert_eq!(summary.estates.len(), 1);
        assert_eq!(summary.estates[0]["land_ident"], 101);
        assert_eq!(summary.status_message, None);
    }

    #[test]
    fn housing_summary_overflow_object_returns_partial_rows_and_status_message() {
        let summary = parse_housing_summary_json(
            r#"{"estates":[{"land_ident":101,"owner_name":"Tester"}],"truncated":true,"total":10,"returned":1,"omitted":9,"message":"Housing summary was truncated to fit the admin IPC payload limit."}"#,
        );

        assert_eq!(summary.estates.len(), 1);
        assert_eq!(summary.estates[0]["land_ident"], 101);
        assert_eq!(
            summary.status_message,
            Some("Housing summary was truncated to fit the admin IPC payload limit.".to_string())
        );
    }

    #[test]
    fn housing_summary_legacy_overflow_error_object_returns_status_message() {
        let summary = parse_housing_summary_json(
            r#"{"error":"housing_summary_ipc_overflow","truncated":true}"#,
        );

        assert!(summary.estates.is_empty());
        assert_eq!(
            summary.status_message,
            Some(
                "Housing summary exceeded the admin IPC payload limit; no estate rows were loaded."
                    .to_string()
            )
        );
    }

    #[test]
    fn housing_summary_error_object_returns_status_message() {
        let summary = parse_housing_summary_json(r#"{"error":"housing_summary_backend_failed"}"#);

        assert!(summary.estates.is_empty());
        assert_eq!(
            summary.status_message,
            Some("Housing summary request failed: housing_summary_backend_failed".to_string())
        );
    }

    #[test]
    fn housing_detail_transport_failure_returns_visible_message() {
        let detail = parse_housing_detail_response(None).pretty_json;

        assert_eq!(
            detail,
            "World server did not respond to housing detail request.".to_string()
        );
    }

    #[test]
    fn housing_detail_json_response_is_pretty_printed() {
        let detail = parse_housing_detail_response(Some(CustomIpcSegment::new(
            CustomIpcData::HousingEstateDetailResponse {
                json: r#"{"land_ident":101,"furniture_counts":{"total":4}}"#.to_string(),
            },
        )))
        .pretty_json;

        assert!(detail.contains("\n  \"land_ident\": 101"));
        assert!(detail.contains("\n  \"furniture_counts\": {"));
    }

    #[test]
    fn housing_import_request_accepts_bare_and_prefixed_export_paths() {
        assert!(matches!(
            build_import_housing_estate_request("estate-123.json"),
            Ok(CustomIpcData::ImportHousingEstate { path }) if path == "housing-exports/estate-123.json"
        ));
        assert!(matches!(
            build_import_housing_estate_request("housing-exports/estate-123.json"),
            Ok(CustomIpcData::ImportHousingEstate { path }) if path == "housing-exports/estate-123.json"
        ));
    }

    #[test]
    fn housing_import_request_trims_whitespace_before_validation() {
        assert!(matches!(
            build_import_housing_estate_request("  estate-123.json \r\n"),
            Ok(CustomIpcData::ImportHousingEstate { path }) if path == "housing-exports/estate-123.json"
        ));
    }

    #[test]
    fn housing_import_request_rejects_parent_traversal() {
        assert!(build_import_housing_estate_request("../estate-123.json").is_err());
        assert!(build_import_housing_estate_request("housing-exports/../estate-123.json").is_err());
    }

    #[test]
    fn housing_redirect_location_preserves_selection_and_encodes_status_message() {
        assert_eq!(
            housing_redirect_location(
                Some(101),
                "success",
                "Updated estate text for 101. Path: housing-exports/estate 101.json",
            ),
            "/housing?land_ident=101&status=success&message=Updated%20estate%20text%20for%20101.%20Path%3A%20housing-exports%2Festate%20101.json"
        );
    }

    #[test]
    fn housing_redirect_after_post_uses_see_other_status_and_location() {
        let response =
            housing_redirect_after_post(None, "error", "Import path is required.").into_response();

        assert_eq!(response.status(), StatusCode::SEE_OTHER);
        assert_eq!(
            response
                .headers()
                .get(header::LOCATION)
                .expect("redirect should set Location")
                .to_str()
                .expect("Location should be valid ASCII"),
            "/housing?status=error&message=Import%20path%20is%20required."
        );
    }

    #[test]
    fn housing_export_response_message_includes_export_path() {
        let message = housing_export_response_message(
            Some(CustomIpcSegment::new(
                CustomIpcData::HousingEstateExported {
                    path: "housing-exports/estate-101.json".to_string(),
                    message: "Exported estate 101.".to_string(),
                },
            )),
            "Unexpected response while exporting estate.",
            "World server did not respond to export estate.",
        );

        assert_eq!(
            message,
            "Exported estate 101. Path: housing-exports/estate-101.json"
        );
    }

    #[test]
    fn housing_query_status_message_uses_query_message_with_status_label() {
        let query = HousingQuery {
            land_ident: Some(101),
            status: Some("success".to_string()),
            message: Some("Updated estate text for 101.".to_string()),
        };

        assert_eq!(
            housing_query_status_message(&query),
            Some("Success: Updated estate text for 101.".to_string())
        );
    }

    #[test]
    fn admin_default_environment_uses_current_directory_template_path() {
        let _lock = CWD_LOCK
            .lock()
            .expect("cwd test lock should not be poisoned");
        let temp_dir = std::env::temp_dir().join(format!(
            "kawari-admin-template-loader-{}",
            std::process::id()
        ));
        let template_dir = temp_dir.join("resources/web/templates");
        fs::create_dir_all(&template_dir).expect("test template directory should be created");
        fs::write(
            template_dir.join("admin_housing.html"),
            "relative-loader-marker",
        )
        .expect("test template should be written");

        let _cwd = CurrentDirGuard::change_to(&temp_dir);
        let environment = setup_default_environment();
        let rendered = environment
            .get_template("admin_housing.html")
            .expect("cwd-relative template should be loaded")
            .render(context! {})
            .expect("test template should render");

        assert_eq!(rendered, "relative-loader-marker");

        drop(_cwd);
        fs::remove_dir_all(temp_dir).expect("test template directory should be cleaned up");
    }

    #[test]
    fn admin_housing_status_message_escapes_template_html() {
        let environment = test_template_environment();
        let template = environment.get_template("admin_housing.html").unwrap();

        let rendered = template
            .render(context! {
                estates => Vec::<serde_json::Value>::new(),
                selected_land_ident => Option::<i64>::None,
                selected_estate => Option::<serde_json::Value>::None,
                selected_detail_json => Option::<String>::None,
                status_message => Some("<script>alert(1)</script>".to_string()),
                name_max_bytes => 20,
                greeting_max_bytes => 192,
            })
            .unwrap();

        assert!(rendered.contains("&lt;script&gt;alert(1)&lt;&#x2f;script&gt;"));
        assert!(!rendered.contains("<script>alert(1)</script>"));
    }

    #[test]
    fn housing_detail_response_extracts_selected_estate_for_editing() {
        let detail = parse_housing_detail_response(Some(CustomIpcSegment::new(
            CustomIpcData::HousingEstateDetailResponse {
                json: r#"{"estate":{"land_ident":101,"estate_name":"Test Estate","greeting":"Welcome."},"furniture_counts":{"total":4},"furniture":[]}"#.to_string(),
            },
        )));

        assert_eq!(
            detail
                .selected_estate
                .as_ref()
                .and_then(|estate| estate["land_ident"].as_i64()),
            Some(101)
        );
        assert_eq!(
            detail
                .selected_estate
                .as_ref()
                .and_then(|estate| estate["estate_name"].as_str()),
            Some("Test Estate")
        );
        assert!(detail.pretty_json.contains("\"greeting\": \"Welcome.\""));
    }

    #[test]
    fn housing_detail_overflow_response_keeps_selected_estate_for_actions() {
        let detail = parse_housing_detail_response(Some(CustomIpcSegment::new(
            CustomIpcData::HousingEstateDetailResponse {
                json: r#"{"error":"housing_detail_ipc_overflow","truncated":true,"estate":{"land_ident":101,"estate_name":"Test Estate","greeting":"Welcome."},"land_ident":101,"furniture_counts":{"total":512},"furniture_omitted":512}"#.to_string(),
            },
        )));

        assert_eq!(
            detail
                .selected_estate
                .as_ref()
                .and_then(|estate| estate["land_ident"].as_i64()),
            Some(101)
        );
        assert!(detail.pretty_json.contains("housing_detail_ipc_overflow"));
    }

    #[test]
    fn housing_update_text_warning_reports_backend_clamping() {
        let warning = update_housing_estate_text_warning(&HousingUpdateTextForm {
            land_ident: 101,
            name: "abcdefghijklmnopqrstu".to_string(),
            greeting: format!("{}끝", "나".repeat(192)),
        });

        assert!(warning.is_some());
        let warning = warning.expect("clamped inputs should produce a warning");
        assert!(warning.contains("20-byte"));
        assert!(warning.contains("192-byte"));
    }
}
