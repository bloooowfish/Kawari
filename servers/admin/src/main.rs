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
    render_housing_page(query.land_ident, None).await
}

async fn reset_housing_furniture(Form(input): Form<HousingLandIdentForm>) -> Html<String> {
    let message = match send_custom_world_packet(CustomIpcSegment::new(
        CustomIpcData::ResetHousingFurniture {
            land_ident: input.land_ident,
        },
    ))
    .await
    {
        Some(response) => match response.data {
            CustomIpcData::HousingEstateImportResult { message } => message,
            _ => "Unexpected response while resetting furniture.".to_string(),
        },
        None => "World server did not respond to reset furniture.".to_string(),
    };

    render_housing_page(Some(input.land_ident), Some(message)).await
}

async fn reset_housing_estate(Form(input): Form<HousingResetEstateForm>) -> Html<String> {
    if input.confirm_reset.as_deref() != Some("on") {
        return render_housing_page(
            Some(input.land_ident),
            Some("Confirmation checkbox is required before resetting an estate.".to_string()),
        )
        .await;
    }

    let message =
        match send_custom_world_packet(CustomIpcSegment::new(CustomIpcData::ResetHousingEstate {
            land_ident: input.land_ident,
        }))
        .await
        {
            Some(response) => match response.data {
                CustomIpcData::HousingEstateImportResult { message } => message,
                _ => "Unexpected response while resetting estate.".to_string(),
            },
            None => "World server did not respond to reset estate.".to_string(),
        };

    render_housing_page(None, Some(message)).await
}

async fn update_housing_text(Form(input): Form<HousingUpdateTextForm>) -> Html<String> {
    let warning = update_housing_estate_text_warning(&input);
    let message = match send_custom_world_packet(CustomIpcSegment::new(
        build_update_housing_estate_text_request(&input),
    ))
    .await
    {
        Some(response) => match response.data {
            CustomIpcData::HousingEstateImportResult { message } => message,
            _ => "Unexpected response while updating estate text.".to_string(),
        },
        None => "World server did not respond to update estate text.".to_string(),
    };

    render_housing_page(
        Some(input.land_ident),
        merge_status_messages(warning, Some(message)),
    )
    .await
}

async fn export_housing_estate(Form(input): Form<HousingLandIdentForm>) -> Html<String> {
    let message =
        match send_custom_world_packet(CustomIpcSegment::new(CustomIpcData::ExportHousingEstate {
            land_ident: input.land_ident,
        }))
        .await
        {
            Some(response) => match response.data {
                CustomIpcData::HousingEstateExported { path, message } => {
                    if path.is_empty() {
                        message
                    } else {
                        format!("{message} Path: {path}")
                    }
                }
                _ => "Unexpected response while exporting estate.".to_string(),
            },
            None => "World server did not respond to export estate.".to_string(),
        };

    render_housing_page(Some(input.land_ident), Some(message)).await
}

async fn import_housing_estate(Form(input): Form<HousingImportForm>) -> Html<String> {
    if input.path.trim().is_empty() {
        return render_housing_page(None, Some("Import path is required.".to_string())).await;
    }

    let request = match build_import_housing_estate_request(&input.path) {
        Ok(request) => request,
        Err(message) => return render_housing_page(None, Some(message)).await,
    };

    let message = match send_custom_world_packet(CustomIpcSegment::new(request)).await {
        Some(response) => match response.data {
            CustomIpcData::HousingEstateImportResult { message } => message,
            _ => "Unexpected response while importing estate.".to_string(),
        },
        None => "World server did not respond to import estate.".to_string(),
    };

    render_housing_page(None, Some(message)).await
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
    use kawari::ipc::kawari::{CustomIpcData, CustomIpcSegment};

    use super::{
        HousingUpdateTextForm, build_import_housing_estate_request, parse_housing_detail_response,
        parse_housing_summary_json, update_housing_estate_text_warning,
    };

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
    fn housing_summary_overflow_object_returns_status_message() {
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
    fn housing_import_request_rejects_parent_traversal() {
        assert!(build_import_housing_estate_request("../estate-123.json").is_err());
        assert!(build_import_housing_estate_request("housing-exports/../estate-123.json").is_err());
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
